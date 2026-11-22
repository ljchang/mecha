//! How much each repository is used (`docs/GITHUB-DASHBOARD-DESIGN.md` §3,
//! §6 view 3, step 5): traffic, release downloads, and the mapping from a
//! repository to the packages it publishes. The registries' counts are
//! [`super::registry`]'s, a pass after the write.
//!
//! None of these is the number the owner asked for (G1), and nothing here
//! pretends one is:
//!
//! - **Traffic is GitHub's fourteen days, kept from the first run on.** Each
//!   read is upserted per day, so the window accumulates into a history that
//!   starts the day the collector first read it — which is why every total
//!   the board shows carries its since-date. A day inside a window GitHub
//!   answered and did not list is a zero, because the answer covered it; a
//!   day no answer covered has no row, and the board counts those as missing,
//!   never as zero.
//! - **Traffic is shown only to a login that can push.** A repository whose
//!   permissions say it cannot is not asked, and one GitHub refuses for it is
//!   logged `traffic_no_access` — a refusal, not a failure. Either way
//!   `traffic_access` says `no_access` and the board shows a dash.
//! - **A clone is not an install.** CI, mirrors and bots clone. The columns
//!   are named for what they count.
//! - **Release downloads are GitHub's own per-asset counts**, sampled once a
//!   day for the newest [`RELEASES_ASKED`] releases. They are all-time
//!   counts, not counts since the collector started; source archives GitHub
//!   generates are not counted at all.
//! - **A package name comes from the repository's own metadata**, read at
//!   the default branch's head, held in memory, and kept only as the name it
//!   yields (checked against the registry's grammar). The owner corrects the
//!   mapping in `~/.mecha/github/packages.toml` ([`read_packages`]), which
//!   the picker ([`super::picker`], step 6) writes; the collector only reads
//!   it.

use super::{
    graphql_errors, split_full_name, Collected, Flow, Gh, GhError, Kind, Problem, Repo, Step, BATCH,
};
use anyhow::Result;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use rusqlite::{params, Connection, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// GitHub's traffic window: the last fourteen days, today's partial day the
/// newest.
pub const TRAFFIC_WINDOW_DAYS: i64 = 14;
/// Repositories whose traffic is read per run (two calls each), the longest
/// unread first. A selection of four hundred is read in three runs, each
/// repository well inside the window, and the calls stay a bounded share of
/// the run's budget and the REST quota.
pub const TRAFFIC_PER_RUN: usize = 150;
/// Releases asked for per repository, newest first, and assets per release.
pub const RELEASES_ASKED: usize = 20;
pub const ASSETS_ASKED: usize = 50;
/// Repositories whose manifests are read per run.
pub const PACKAGE_SCANS_PER_RUN: usize = 50;
/// A repository's manifests are read again when its default branch has been
/// pushed since, or after this long whatever happened.
pub const PACKAGE_RESCAN_DAYS: i64 = 30;
/// The most of one manifest read; a larger one is not parsed, and says so.
const MANIFEST_BYTES_MAX: usize = 256 * 1024;

/// The releases field of the batch's repository fragment (`batch_query`).
pub(super) const RELEASE_FIELDS: &str = "releases(first: 20, orderBy: {field: CREATED_AT, \
     direction: DESC}) { totalCount nodes { databaseId tagName releaseAssets(first: 50) { \
     totalCount nodes { databaseId name downloadCount } } } }";

// ---- traffic ----

/// One traffic series as GitHub gave it: its window's total and uniques, and
/// each day it listed.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Series {
    pub total: Option<i64>,
    pub uniques: Option<i64>,
    pub days: BTreeMap<NaiveDate, (Option<i64>, Option<i64>)>,
}

/// What one repository's traffic said this run.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "access")]
pub enum TrafficRead {
    /// Both series answered; `from..=to` is the window they cover.
    Read {
        clones: Series,
        views: Series,
        from: NaiveDate,
        to: NaiveDate,
    },
    /// The login cannot push to it: not asked, or refused. Our words only.
    NoAccess { detail: String },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Traffic {
    pub repo: String,
    pub read: TrafficRead,
}

/// `clones` or `views` from the traffic API's answer. `None` for an answer
/// that is not that shape — one malformed day makes the whole answer
/// unreadable, never a partial series written as if whole.
fn parse_series(body: &str, key: &str) -> Option<Series> {
    let v: Value = serde_json::from_str(body).ok()?;
    let mut days = BTreeMap::new();
    for e in v.get(key)?.as_array()? {
        let day = e
            .get("timestamp")?
            .as_str()?
            .get(..10)?
            .parse::<NaiveDate>()
            .ok()?;
        days.insert(
            day,
            (
                e.get("count").and_then(Value::as_i64),
                e.get("uniques").and_then(Value::as_i64),
            ),
        );
    }
    Some(Series {
        total: v.get("count").and_then(Value::as_i64),
        uniques: v.get("uniques").and_then(Value::as_i64),
        days,
    })
}

/// When each repository's traffic was last asked about, from the store.
fn prior_traffic(db: &Path) -> BTreeMap<String, String> {
    let Ok(conn) = Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return BTreeMap::new();
    };
    let Ok(mut stmt) = conn.prepare("SELECT repo, checked_at FROM traffic_access") else {
        return BTreeMap::new();
    };
    stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map(|rows| {
            rows.filter_map(Result::ok)
                .map(|(r, at)| (r.to_ascii_lowercase(), at))
                .collect()
        })
        .unwrap_or_default()
}

/// Read the selected repositories' traffic, at most [`TRAFFIC_PER_RUN`] of
/// them, never-read first and then the longest since read.
pub(super) fn traffic_step(
    gh: &Gh,
    db: &Path,
    selected: &[Repo],
    now: DateTime<Utc>,
    c: &mut Collected,
) -> Flow {
    let last = prior_traffic(db);
    let mut order: Vec<&Repo> = selected.iter().collect();
    // `None` sorts first: a repository never read goes before any read one.
    order.sort_by_key(|r| last.get(&r.full_name.to_ascii_lowercase()).cloned());
    let to = now.date_naive();
    let from = to - Duration::days(TRAFFIC_WINDOW_DAYS - 1);
    for r in order.into_iter().take(TRAFFIC_PER_RUN) {
        if r.push == Some(false) {
            c.traffic.push(Traffic {
                repo: r.full_name.clone(),
                read: TrafficRead::NoAccess {
                    detail: "the login's permissions say it cannot push here, and GitHub shows \
                             traffic only to accounts that can; not asked"
                        .into(),
                },
            });
            continue;
        }
        let mut got: Vec<Series> = Vec::new();
        for key in ["clones", "views"] {
            let path = format!("repos/{}/traffic/{key}", r.full_name);
            match gh.rest(&["api", path.as_str()]) {
                Ok(body) => match parse_series(&body, key) {
                    Some(s) => got.push(s),
                    None => {
                        c.problems.push(Problem {
                            kind: Kind::Failed,
                            step: Step::Traffic,
                            repo: Some(r.full_name.clone()),
                            detail: format!("its traffic {key} answer could not be read"),
                        });
                        break;
                    }
                },
                Err(GhError::NoAccess(d)) => {
                    c.traffic.push(Traffic {
                        repo: r.full_name.clone(),
                        read: TrafficRead::NoAccess {
                            detail: format!("GitHub refused its traffic: {d}"),
                        },
                    });
                    c.problems
                        .push(GhError::NoAccess(d).problem(Step::Traffic, Some(&r.full_name)));
                    break;
                }
                Err(e) => {
                    let stop = matches!(
                        e,
                        GhError::Auth(_) | GhError::Missing(_) | GhError::OutOfTime
                    );
                    let limited = matches!(e, GhError::RateLimited(_));
                    let repo = (!limited).then_some(r.full_name.as_str());
                    c.problems.push(e.problem(Step::Traffic, repo));
                    if stop {
                        return Flow::Stop;
                    }
                    if limited {
                        // The REST quota is not spent further; what is not
                        // read keeps its history and is first next run.
                        return Flow::Go;
                    }
                    break;
                }
            }
        }
        // Both series, or nothing written: one missing has its problem (or
        // its refusal) already, and a day with clones and no views is not a
        // day to write as if whole.
        if let (Some(views), Some(clones), None) = (got.pop(), got.pop(), got.pop()) {
            c.traffic.push(Traffic {
                repo: r.full_name.clone(),
                read: TrafficRead::Read {
                    clones,
                    views,
                    from,
                    to,
                },
            });
        }
    }
    Flow::Go
}

// ---- releases ----

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Asset {
    pub asset_id: i64,
    pub release_id: Option<i64>,
    pub tag: Option<String>,
    pub name: String,
    /// GitHub's count for the asset, since it was uploaded. `None` when the
    /// answer did not give one, never a zero.
    pub downloads: Option<i64>,
}

/// A repository's newest releases' assets, as one answer gave them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Releases {
    /// How many releases the repository has.
    pub total: Option<i64>,
    pub assets: Vec<Asset>,
    /// Every release and every asset was in the answer: a sum over these is
    /// the repository's whole count, not its newest releases' alone.
    pub complete: bool,
}

/// The `releases` field of one repository's GraphQL object. `None` when the
/// object has none, which is unknown, not "no releases".
pub(super) fn parse_releases(d: &Value) -> Option<Releases> {
    let rel = d.get("releases").filter(|v| v.is_object())?;
    let nodes = rel.get("nodes").and_then(Value::as_array)?;
    let total = rel.get("totalCount").and_then(Value::as_i64);
    let mut complete = total.is_some_and(|t| t <= nodes.len() as i64);
    let mut assets = Vec::new();
    for n in nodes {
        let release_id = n.get("databaseId").and_then(Value::as_i64);
        let tag = n.get("tagName").and_then(Value::as_str).map(str::to_string);
        let Some(ra) = n.get("releaseAssets").filter(|v| v.is_object()) else {
            complete = false;
            continue;
        };
        let list = ra
            .get("nodes")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if ra
            .get("totalCount")
            .and_then(Value::as_i64)
            .is_none_or(|t| t > list.len() as i64)
        {
            complete = false;
        }
        for a in list {
            let (Some(asset_id), Some(name)) = (
                a.get("databaseId").and_then(Value::as_i64),
                a.get("name").and_then(Value::as_str),
            ) else {
                complete = false;
                continue;
            };
            assets.push(Asset {
                asset_id,
                release_id,
                tag: tag.clone(),
                name: name.to_string(),
                downloads: a.get("downloadCount").and_then(Value::as_i64),
            });
        }
    }
    Some(Releases {
        total,
        assets,
        complete,
    })
}

// ---- package names ----

/// A registry a package name is looked up in. A wire format: written to the
/// store and to `packages.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Registry {
    Pypi,
    Npm,
    Conda,
}

impl Registry {
    pub const ALL: [Registry; 3] = [Registry::Pypi, Registry::Npm, Registry::Conda];

    pub fn as_str(self) -> &'static str {
        match self {
            Registry::Pypi => "pypi",
            Registry::Npm => "npm",
            Registry::Conda => "conda",
        }
    }

    pub fn parse(s: &str) -> Option<Registry> {
        Registry::ALL.into_iter().find(|r| r.as_str() == s)
    }
}

/// A package name in its registry's canonical form, or `None` when it is
/// outside the registry's grammar. Checked on everything — a manifest's
/// text or the owner's file — before it is stored, and again before it is
/// put in a registry's address ([`super::registry`]), so no name can carry
/// a path, a query or an escape.
///
/// - PyPI: PEP 508's name, normalised as PEP 503 says (lower case, each run
///   of `-`, `_`, `.` one `-`), which is the form pypistats is asked in.
/// - npm: lower case, `[a-z0-9-._~]`, not starting with `.` or `_`, at most
///   214 characters, optionally `@scope/` before it.
/// - conda: lower case `[a-z0-9_.-]`, starting with a letter or digit.
pub fn canonical_name(registry: Registry, name: &str) -> Option<String> {
    let name = name.trim();
    match registry {
        Registry::Pypi => {
            let ok = (1..=100).contains(&name.len())
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
                && name.as_bytes()[0].is_ascii_alphanumeric()
                && name.as_bytes()[name.len() - 1].is_ascii_alphanumeric();
            if !ok {
                return None;
            }
            let mut out = String::with_capacity(name.len());
            let mut sep = false;
            for ch in name.chars() {
                if matches!(ch, '-' | '_' | '.') {
                    sep = true;
                } else {
                    if sep {
                        out.push('-');
                        sep = false;
                    }
                    out.push(ch.to_ascii_lowercase());
                }
            }
            Some(out)
        }
        Registry::Npm => {
            let part = |p: &str| {
                !p.is_empty()
                    && !p.starts_with('.')
                    && !p.starts_with('_')
                    && p.bytes().all(|b| {
                        b.is_ascii_lowercase()
                            || b.is_ascii_digit()
                            || matches!(b, b'-' | b'.' | b'_' | b'~')
                    })
            };
            let ok = (1..=214).contains(&name.len())
                && match name.strip_prefix('@') {
                    Some(scoped) => scoped
                        .split_once('/')
                        .is_some_and(|(scope, pkg)| part(scope) && part(pkg)),
                    None => part(name),
                };
            ok.then(|| name.to_string())
        }
        Registry::Conda => {
            let lower = name.to_ascii_lowercase();
            let ok = (1..=100).contains(&lower.len())
                && lower.as_bytes()[0].is_ascii_alphanumeric()
                && lower.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'.' | b'-')
                });
            ok.then_some(lower)
        }
    }
}

/// The manifests asked for, by GraphQL alias and path at the default branch.
/// `setup.py` is not among them: it is a program, and its name is whatever
/// it computes.
pub const MANIFESTS: [(&str, &str); 5] = [
    ("pyproject", "pyproject.toml"),
    ("setupcfg", "setup.cfg"),
    ("packagejson", "package.json"),
    ("recipe", "recipe/meta.yaml"),
    ("condarecipe", "conda.recipe/meta.yaml"),
];

/// `pyproject.toml`: PEP 621's `[project] name`, else Poetry's
/// `[tool.poetry] name`.
pub fn pyproject_name(text: &str) -> Option<String> {
    let t: toml::Table = toml::from_str(text).ok()?;
    let of = |v: Option<&toml::Value>| v.and_then(|v| v.get("name")?.as_str().map(str::to_string));
    of(t.get("project"))
        .or_else(|| of(t.get("tool").and_then(|t| t.get("poetry"))))
        .and_then(|n| canonical_name(Registry::Pypi, &n))
}

/// `setup.cfg`: `name` in its `[metadata]` section.
pub fn setup_cfg_name(text: &str) -> Option<String> {
    let mut inside = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            inside = l == "[metadata]";
            continue;
        }
        if !inside || l.starts_with('#') || l.starts_with(';') {
            continue;
        }
        if let Some((k, v)) = l.split_once('=').or_else(|| l.split_once(':')) {
            if k.trim() == "name" {
                return canonical_name(Registry::Pypi, v.trim());
            }
        }
    }
    None
}

/// `package.json`: its `name` — unless `private` is true, which npm refuses
/// to publish, so there is nothing on the registry to count.
pub fn package_json_name(text: &str) -> Option<String> {
    let v: Value = serde_json::from_str(text).ok()?;
    if v.get("private").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    canonical_name(Registry::Npm, v.get("name")?.as_str()?)
}

/// A conda recipe's `meta.yaml`: `name:` under `package:`, as a literal or
/// as `{{ name }}` (with `|lower`) after `{% set name = "..." %}`, the form
/// conda-forge's recipes use. Anything else Jinja computes is not guessed.
pub fn conda_recipe_name(text: &str) -> Option<String> {
    let unquote = |s: &str| s.trim().trim_matches(|c| c == '"' || c == '\'').to_string();
    let mut set_name: Option<String> = None;
    for line in text.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("{%").and_then(|r| r.strip_suffix("%}")) {
            let rest = rest.trim().trim_start_matches('-').trim();
            if let Some(assign) = rest.strip_prefix("set ") {
                if let Some((k, v)) = assign.split_once('=') {
                    if k.trim() == "name" {
                        set_name = Some(unquote(v.trim().trim_end_matches('-')));
                    }
                }
            }
        }
    }
    let mut in_package = false;
    for line in text.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if !indented {
            in_package = line.trim_end() == "package:";
            continue;
        }
        if !in_package {
            continue;
        }
        let Some(v) = line.trim().strip_prefix("name:") else {
            continue;
        };
        let v = unquote(v);
        let name = match v.strip_prefix("{{").and_then(|r| r.strip_suffix("}}")) {
            Some(expr) => {
                let expr: String = expr.chars().filter(|c| !c.is_whitespace()).collect();
                match expr.as_str() {
                    "name" | "name|lower" => set_name.clone()?,
                    _ => return None,
                }
            }
            None if v.contains("{{") || v.contains("{%") => return None,
            None => v,
        };
        return canonical_name(Registry::Conda, &name);
    }
    None
}

/// A name found in a repository's manifest.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Detected {
    pub registry: Registry,
    pub name: String,
    pub manifest: &'static str,
}

/// One repository's manifests, read.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Scan {
    pub repo: String,
    /// The `pushed_at` the read was of, so a later push asks again.
    pub pushed_at: Option<String>,
    /// Empty when no manifest named a package — which is said, as a scan
    /// that found none, never left looking unscanned.
    pub detected: Vec<Detected>,
    /// Manifests that were there and gave no name, each with why, in our
    /// words (`pyproject.toml: no name`, `package.json: too large`).
    pub notes: Vec<String>,
}

/// A manifest's reader: its text to the name it gives, if any.
type NameOf = fn(&str) -> Option<String>;

/// The names one repository's GraphQL object of manifests yields. PyPI's is
/// `pyproject.toml`'s, else `setup.cfg`'s; conda's is `recipe/`'s, else
/// `conda.recipe/`'s.
pub fn scan_of(repo: &Repo, d: &Value) -> Scan {
    let mut notes = Vec::new();
    let mut text = |alias: &str, path: &str| -> Option<String> {
        let blob = d.get(alias).filter(|v| v.is_object())?;
        let t = blob.get("text").and_then(Value::as_str);
        if blob.get("isTruncated").and_then(Value::as_bool) == Some(true)
            || t.is_some_and(|t| t.len() > MANIFEST_BYTES_MAX)
        {
            notes.push(format!("{path}: too large"));
            return None;
        }
        match t {
            Some(t) => Some(t.to_string()),
            None => {
                notes.push(format!("{path}: not text"));
                None
            }
        }
    };
    let files: BTreeMap<&str, Option<String>> = MANIFESTS
        .iter()
        .map(|(alias, path)| (*path, text(alias, path)))
        .collect();
    let mut detected = Vec::new();
    let mut found = |registry: Registry, candidates: &[(&'static str, NameOf)]| {
        for (path, parse) in candidates {
            if let Some(Some(t)) = files.get(path) {
                match parse(t) {
                    Some(name) => {
                        detected.push(Detected {
                            registry,
                            name,
                            manifest: path,
                        });
                        return;
                    }
                    None => notes.push(format!("{path}: no {} name", registry.as_str())),
                }
            }
        }
    };
    found(
        Registry::Pypi,
        &[
            ("pyproject.toml", pyproject_name),
            ("setup.cfg", setup_cfg_name),
        ],
    );
    found(Registry::Npm, &[("package.json", package_json_name)]);
    found(
        Registry::Conda,
        &[
            ("recipe/meta.yaml", conda_recipe_name),
            ("conda.recipe/meta.yaml", conda_recipe_name),
        ],
    );
    Scan {
        repo: repo.full_name.clone(),
        pushed_at: repo.pushed_at.clone(),
        detected,
        notes,
    }
}

fn scan_query(n: usize) -> String {
    let mut q = String::from("query(");
    for i in 0..n {
        if i > 0 {
            q.push_str(", ");
        }
        q.push_str(&format!("$o{i}: String!, $n{i}: String!"));
    }
    q.push_str(") {\n");
    for i in 0..n {
        q.push_str(&format!(
            "  r{i}: repository(owner: $o{i}, name: $n{i}) {{ ...M }}\n"
        ));
    }
    q.push_str("}\nfragment M on Repository {\n");
    for (alias, path) in MANIFESTS {
        q.push_str(&format!(
            "  {alias}: object(expression: \"HEAD:{path}\") {{ ... on Blob {{ text isTruncated }} }}\n"
        ));
    }
    q.push_str("}\n");
    q
}

/// When each repository's manifests were last read, and of which push.
fn prior_scans(db: &Path) -> BTreeMap<String, (String, Option<String>)> {
    let Ok(conn) = Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return BTreeMap::new();
    };
    let Ok(mut stmt) = conn.prepare("SELECT repo, at, pushed_at FROM package_scans") else {
        return BTreeMap::new();
    };
    stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    })
    .map(|rows| {
        rows.filter_map(Result::ok)
            .map(|(r, at, p)| (r.to_ascii_lowercase(), (at, p)))
            .collect()
    })
    .unwrap_or_default()
}

fn scan_due(prior: Option<&(String, Option<String>)>, r: &Repo, now: DateTime<Utc>) -> bool {
    match prior {
        None => true,
        Some((at, pushed)) => {
            *pushed != r.pushed_at
                || super::parse_ts(at).is_none_or(|t| now - t > Duration::days(PACKAGE_RESCAN_DAYS))
        }
    }
}

/// Read the manifests of the selected repositories that are due, at most
/// [`PACKAGE_SCANS_PER_RUN`], by GraphQL a batch at a time.
pub(super) fn scan_step(
    gh: &Gh,
    db: &Path,
    selected: &[Repo],
    now: DateTime<Utc>,
    c: &mut Collected,
) -> Flow {
    let prior = prior_scans(db);
    let key = |r: &Repo| prior.get(&r.full_name.to_ascii_lowercase());
    let mut due: Vec<&Repo> = selected
        .iter()
        .filter(|r| scan_due(key(r), r, now))
        .collect();
    due.sort_by_key(|r| key(r).map(|(at, _)| at.clone()));
    due.truncate(PACKAGE_SCANS_PER_RUN);
    for chunk in due.chunks(BATCH) {
        let vars: Vec<(String, String)> = chunk
            .iter()
            .enumerate()
            .flat_map(|(i, r)| {
                [
                    (format!("o{i}"), r.owner.clone()),
                    (format!("n{i}"), r.name.clone()),
                ]
            })
            .collect();
        let answer = match gh.graphql(&scan_query(chunk.len()), &vars) {
            Ok(v) => v,
            Err(e) => {
                let stop = matches!(
                    e,
                    GhError::Auth(_) | GhError::Missing(_) | GhError::OutOfTime
                );
                if stop || matches!(e, GhError::RateLimited(_)) {
                    c.problems.push(e.problem(Step::Packages, None));
                    return if stop { Flow::Stop } else { Flow::Go };
                }
                for r in chunk {
                    c.problems
                        .push(e.problem(Step::Packages, Some(&r.full_name)));
                }
                continue;
            }
        };
        let errors: Vec<_> = graphql_errors(&answer).collect();
        for (i, r) in chunk.iter().enumerate() {
            let alias = format!("r{i}");
            match answer.get("data").and_then(|d| d.get(&alias)) {
                Some(d) if d.is_object() => c.scans.push(scan_of(r, d)),
                _ => {
                    let why = errors
                        .iter()
                        .find(|(p, _, _)| p.as_deref() == Some(alias.as_str()))
                        .map(|(_, _, m)| m.clone())
                        .unwrap_or_else(|| "no data for it in the GraphQL answer".into());
                    c.problems.push(Problem {
                        kind: Kind::Failed,
                        step: Step::Packages,
                        repo: Some(r.full_name.clone()),
                        detail: format!("its manifests could not be read: {why}"),
                    });
                }
            }
        }
    }
    Flow::Go
}

// ---- the owner's corrections ----

/// `<home>/github/packages.toml` — the owner's corrections to the package
/// mapping. The picker ([`super::picker`]) writes it; the collector only
/// reads it.
pub fn packages_path(home: &Path) -> PathBuf {
    super::dir(home).join("packages.toml")
}

/// `packages.toml`:
///
/// ```toml
/// # The name a repository publishes under, where the manifest says
/// # otherwise or says nothing.
/// [[package]]
/// repo = "example-lab/tool-a"
/// registry = "pypi"
/// name = "tool-a-py"
///
/// # A name detected that is not this repository's to count.
/// [[package]]
/// repo = "example-lab/tool-b"
/// registry = "npm"
/// none = true
/// ```
///
/// A line decides its `(repo, registry)` over whatever was detected, either
/// way; registries a line does not name keep what was detected.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageFile {
    #[serde(default, rename = "package")]
    pub packages: Vec<PackageRule>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageRule {
    pub repo: String,
    pub registry: Registry,
    /// The name to count, canonicalised on read. Exactly one of `name` and
    /// `none = true`.
    #[serde(default)]
    pub name: Option<String>,
    /// This repository publishes nothing to this registry: ignore what is
    /// detected.
    #[serde(default)]
    pub none: bool,
}

#[derive(Debug, Clone)]
pub enum PackagesRead {
    Absent,
    /// It is there and cannot be used.
    Invalid(String),
    Read(Vec<PackageRule>),
}

pub fn read_packages(path: &Path) -> PackagesRead {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return PackagesRead::Absent,
        Err(e) => return PackagesRead::Invalid(format!("{}: {e}", path.display())),
    };
    match parse_packages(&text) {
        Ok(rules) => PackagesRead::Read(rules),
        Err(e) => PackagesRead::Invalid(format!("{}: {e}", path.display())),
    }
}

pub fn parse_packages(text: &str) -> Result<Vec<PackageRule>, String> {
    let f: PackageFile = toml::from_str(text).map_err(|e| e.to_string())?;
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(f.packages.len());
    for mut r in f.packages {
        if split_full_name(&r.repo).is_none() {
            return Err(format!(
                "[[package]] repo {:?} is not owner/name in GitHub's grammar",
                r.repo
            ));
        }
        match (&r.name, r.none) {
            (Some(n), false) => match canonical_name(r.registry, n) {
                Some(c) => r.name = Some(c),
                None => {
                    return Err(format!(
                        "[[package]] {:?} for {}: {n:?} is not a {} package name",
                        r.repo,
                        r.registry.as_str(),
                        r.registry.as_str()
                    ))
                }
            },
            (None, true) => {}
            (Some(_), true) => {
                return Err(format!(
                    "[[package]] {:?} for {}: a name and none = true together say two things",
                    r.repo,
                    r.registry.as_str()
                ))
            }
            (None, false) => {
                return Err(format!(
                    "[[package]] {:?} for {}: needs a name, or none = true",
                    r.repo,
                    r.registry.as_str()
                ))
            }
        }
        if !seen.insert((r.repo.to_ascii_lowercase(), r.registry)) {
            return Err(format!(
                "[[package]] {:?} for {} is listed twice",
                r.repo,
                r.registry.as_str()
            ));
        }
        out.push(r);
    }
    Ok(out)
}

// ---- the store ----

/// Write one run's usage, inside [`super::record`]'s transaction.
pub(super) fn record(tx: &Transaction<'_>, c: &Collected, run: &str, day: &str) -> Result<()> {
    for t in &c.traffic {
        match &t.read {
            TrafficRead::NoAccess { detail } => {
                tx.execute(
                    "INSERT INTO traffic_access (repo, state, checked_at, detail)
                     VALUES (?1, 'no_access', ?2, ?3)
                     ON CONFLICT (repo) DO UPDATE SET state = 'no_access',
                       checked_at = excluded.checked_at, detail = excluded.detail,
                       window_clones = NULL, window_clone_uniques = NULL,
                       window_views = NULL, window_view_uniques = NULL",
                    params![t.repo, run, detail],
                )?;
            }
            TrafficRead::Read {
                clones,
                views,
                from,
                to,
            } => {
                // Every day the answers cover: the window, and any day either
                // listed outside it.
                let first = [Some(*from), clones.days.keys().next().copied()]
                    .into_iter()
                    .chain([views.days.keys().next().copied()])
                    .flatten()
                    .min()
                    .unwrap_or(*from);
                let last = [Some(*to), clones.days.keys().last().copied()]
                    .into_iter()
                    .chain([views.days.keys().last().copied()])
                    .flatten()
                    .max()
                    .unwrap_or(*to);
                let mut d = first;
                while d <= last {
                    let given = |s: &Series| s.days.get(&d).and_then(|(n, u)| n.map(|n| (n, *u)));
                    let (gc, gv) = (given(clones), given(views));
                    // A day GitHub listed: its numbers, the newest answer's
                    // (a day in progress grows; a past day's is final). A day
                    // of the window it did not list: zero, as its answer
                    // says — but never over a number an earlier answer gave.
                    tx.execute(
                        "INSERT INTO traffic (repo, day, clones, clone_uniques, views, view_uniques, at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                         ON CONFLICT (repo, day) DO UPDATE SET
                           clones = CASE WHEN ?8 THEN excluded.clones ELSE coalesce(traffic.clones, excluded.clones) END,
                           clone_uniques = CASE WHEN ?8 THEN excluded.clone_uniques ELSE coalesce(traffic.clone_uniques, excluded.clone_uniques) END,
                           views = CASE WHEN ?9 THEN excluded.views ELSE coalesce(traffic.views, excluded.views) END,
                           view_uniques = CASE WHEN ?9 THEN excluded.view_uniques ELSE coalesce(traffic.view_uniques, excluded.view_uniques) END,
                           at = excluded.at",
                        params![
                            t.repo,
                            d.format("%Y-%m-%d").to_string(),
                            gc.map_or(0, |g| g.0),
                            gc.map_or(Some(0), |g| g.1),
                            gv.map_or(0, |g| g.0),
                            gv.map_or(Some(0), |g| g.1),
                            run,
                            gc.is_some(),
                            gv.is_some()
                        ],
                    )?;
                    d += Duration::days(1);
                }
                tx.execute(
                    "INSERT INTO traffic_access (repo, state, checked_at, detail, window_clones,
                       window_clone_uniques, window_views, window_view_uniques)
                     VALUES (?1, 'read', ?2, NULL, ?3, ?4, ?5, ?6)
                     ON CONFLICT (repo) DO UPDATE SET state = 'read',
                       checked_at = excluded.checked_at, detail = NULL,
                       window_clones = excluded.window_clones,
                       window_clone_uniques = excluded.window_clone_uniques,
                       window_views = excluded.window_views,
                       window_view_uniques = excluded.window_view_uniques",
                    params![
                        t.repo,
                        run,
                        clones.total,
                        clones.uniques,
                        views.total,
                        views.uniques
                    ],
                )?;
            }
        }
    }

    // Releases: the day's latest answer stands for the day.
    for r in &c.repos {
        let Some(rel) = &r.releases else { continue };
        tx.execute(
            "DELETE FROM releases WHERE repo = ?1 AND day = ?2",
            params![r.repo, day],
        )?;
        for a in &rel.assets {
            tx.execute(
                "INSERT OR REPLACE INTO releases (repo, day, asset_id, release_id, tag, asset, downloads, at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![r.repo, day, a.asset_id, a.release_id, a.tag, a.name, a.downloads, run],
            )?;
        }
        tx.execute(
            "INSERT OR REPLACE INTO release_reads (repo, day, releases, assets, complete, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                r.repo,
                day,
                rel.total,
                rel.assets.len() as i64,
                rel.complete,
                run
            ],
        )?;
    }

    // What the manifests said, for the repositories read this run.
    for s in &c.scans {
        tx.execute(
            "INSERT OR REPLACE INTO package_scans (repo, at, pushed_at, found, notes)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                s.repo,
                run,
                s.pushed_at,
                s.detected.len() as i64,
                (!s.notes.is_empty()).then(|| s.notes.join("; "))
            ],
        )?;
        tx.execute(
            "DELETE FROM package_detected WHERE repo = ?1",
            params![s.repo],
        )?;
        for d in &s.detected {
            tx.execute(
                "INSERT INTO package_detected (repo, registry, name, manifest, at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![s.repo, d.registry.as_str(), d.name, d.manifest, run],
            )?;
        }
    }

    // The mapping the registries are asked about: the owner's line decides
    // its (repo, registry) either way, else what was detected. Rebuilt whole
    // each run, so a line removed from the file stops counting — and left
    // as it was when the file cannot be used, never rebuilt without it.
    if let Some(rules) = &c.package_rules {
        tx.execute("DELETE FROM packages", [])?;
        tx.execute(
            "INSERT INTO packages (repo, registry, name, source, manifest, detected_name, at)
             SELECT repo, registry, name, 'detected', manifest, name, ?1 FROM package_detected",
            params![run],
        )?;
        for rule in rules {
            // The inventory's spelling, so a line written in another case is
            // the same repository's row.
            let repo: String = tx
                .query_row(
                    "SELECT repo FROM inventory WHERE lower(repo) = lower(?1)
                     UNION ALL SELECT repo FROM package_detected WHERE lower(repo) = lower(?1)
                     LIMIT 1",
                    params![rule.repo],
                    |r| r.get(0),
                )
                .unwrap_or_else(|_| rule.repo.clone());
            let name = if rule.none { None } else { rule.name.clone() };
            tx.execute(
                "INSERT INTO packages (repo, registry, name, source, manifest, detected_name, at)
                 VALUES (?1, ?2, ?3, 'owner', 'packages.toml',
                   (SELECT name FROM package_detected WHERE repo = ?1 AND registry = ?2), ?4)
                 ON CONFLICT (repo, registry) DO UPDATE SET name = excluded.name,
                   source = 'owner', manifest = 'packages.toml', at = excluded.at",
                params![repo, rule.registry.as_str(), name, run],
            )?;
        }
    }
    Ok(())
}
