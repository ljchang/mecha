//! The package registries' download counts (`docs/GITHUB-DASHBOARD-DESIGN.md`
//! §3, step 5): where installs are counted, for the names the package mapping
//! holds ([`super::usage`]).
//!
//! **A fixed fetch, with no model and no credential.** The collector is a
//! command, not an agent run: it has no tool surface, no registry of tools
//! and no interlock to consult, because nothing in it lets a model choose a
//! call. This pass is the same shape as its `gh` calls — the host is one of
//! two constants here, the path is built from a name that has passed the
//! registry's grammar twice (when stored, and here), and the request carries
//! no token, no cookie and no header naming the owner. A redirect is not
//! followed: a registry that answers "look elsewhere" is a destination this
//! code did not choose, and is logged as a failure.
//!
//! **A private repository's name is never sent.** A name detected in a
//! private repository's manifest is the owner's private information, and
//! asking a third-party service about it would publish that it exists. Only
//! an owner-set name (`packages.toml`) for a private repository is asked
//! about — the owner's act, not the collector's inference.
//!
//! **Refused and failed stay different findings** (§3): a rate limit or a
//! refusal of an anonymous client is `registry_refused` and stops that
//! registry for the run; no answer, a server error or an answer that does
//! not parse is `registry_failed`. A registry that says it has no such
//! package (404) is neither — it is an answer, kept in `registry_checks` as
//! `not_found`, and the board says "not found" with a dash beside it.
//!
//! Which registries: PyPI (through pypistats.org, a third-party service:
//! PyPI's own JSON API no longer reports downloads) and npm (its downloads
//! API). Conda names are detected and mapped, and not yet counted (the
//! design's §7 step 5 says what remains); container pulls are not built.
//!
//! A pass after the collection is written, like the summaries (§5): nothing
//! it does can cost the board its tables, and its problems never fail the
//! unit — a third-party service being down is a finding, not a broken
//! collector.

use super::usage::{canonical_name, Registry};
use super::{open_store, ts, Kind, Problem, Step};
use anyhow::Result;
use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{params, Connection};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant};

/// Packages asked about per run, the longest unchecked first.
pub const PER_RUN: usize = 60;
/// The pass's whole allowance, after the collection's `RUN_BUDGET` and
/// before the summaries' (`the_budgets_fit_inside_the_units_start_timeout`).
pub const BUDGET: Duration = Duration::from_secs(60);
/// The most any one registry call may take.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(15);
/// The most of one registry answer read: a month of daily counts is a few
/// kilobytes, pypistats' half-year a few tens.
pub const BODY_BYTES_MAX: usize = 1024 * 1024;

const PYPISTATS: &str = "https://pypistats.org/api/packages/";
const NPM_DOWNLOADS: &str = "https://api.npmjs.org/downloads/range/last-month/";

/// The address a registry's daily counts are asked at, or `None` for a
/// registry that is not counted, or a name outside its grammar.
pub fn url(registry: Registry, name: &str) -> Option<String> {
    let name = canonical_name(registry, name)?;
    match registry {
        Registry::Pypi => Some(format!("{PYPISTATS}{name}/overall?mirrors=false")),
        Registry::Npm => Some(format!("{NPM_DOWNLOADS}{name}")),
        Registry::Conda => None,
    }
}

/// How the pass reaches a registry: an HTTP GET, its status and its body.
/// `Err` is no answer at all, in our words.
#[async_trait::async_trait]
pub trait Fetch: Send + Sync {
    async fn get(&self, url: &str) -> Result<(u16, String), String>;
}

/// The real one: no credential, no cookies, no redirects, a generic agent.
pub struct Http {
    client: reqwest::Client,
}

impl Http {
    pub fn new() -> Result<Http> {
        Ok(Http {
            client: reqwest::Client::builder()
                .timeout(CALL_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .user_agent("mecha-github-collect")
                .build()?,
        })
    }
}

#[async_trait::async_trait]
impl Fetch for Http {
    async fn get(&self, url: &str) -> Result<(u16, String), String> {
        let mut resp = self.client.get(url).send().await.map_err(|e| {
            if e.is_timeout() {
                "no answer in time".to_string()
            } else {
                "no answer".to_string()
            }
        })?;
        let status = resp.status().as_u16();
        // Capped as it is read: an answer past the cap is refused whole.
        let mut body = Vec::new();
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|_| "the answer broke off".to_string())?
        {
            if body.len() + chunk.len() > BODY_BYTES_MAX {
                return Err(format!(
                    "an answer larger than {} KiB",
                    BODY_BYTES_MAX / 1024
                ));
            }
            body.extend_from_slice(&chunk);
        }
        Ok((status, String::from_utf8_lossy(&body).into_owned()))
    }
}

/// What one registry said about one name.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    /// Downloads per day, as the registry gave them.
    Counted(Vec<(NaiveDate, i64)>),
    /// The registry has no such package: the mapping names something that
    /// is not published there. An answer, not a problem.
    NotFound,
    /// A rate limit, or a refusal of an anonymous client.
    Refused(String),
    Failed(String),
}

impl Answer {
    fn status(&self) -> &'static str {
        match self {
            Answer::Counted(_) => "counted",
            Answer::NotFound => "not_found",
            Answer::Refused(_) => "refused",
            Answer::Failed(_) => "failed",
        }
    }
}

/// Classify one answer by its status first, then its shape. The detail is
/// ours: a status, never the body.
pub fn classify(registry: Registry, status: u16, body: &str) -> Answer {
    match status {
        200 => match parse(registry, body) {
            Some(days) => Answer::Counted(days),
            None => Answer::Failed("an answer that did not parse".into()),
        },
        404 => Answer::NotFound,
        429 => Answer::Refused("HTTP 429, a rate limit".into()),
        401 | 403 => Answer::Refused(format!("HTTP {status}, the client was refused")),
        s => Answer::Failed(format!("HTTP {s}")),
    }
}

fn parse(registry: Registry, body: &str) -> Option<Vec<(NaiveDate, i64)>> {
    let v: Value = serde_json::from_str(body).ok()?;
    let day = |s: &Value| s.as_str()?.get(..10)?.parse::<NaiveDate>().ok();
    let mut out = Vec::new();
    match registry {
        // {"data": [{"category": "without_mirrors", "date": "…", "downloads": n}], …}
        Registry::Pypi => {
            for e in v.get("data")?.as_array()? {
                if e.get("category")
                    .and_then(Value::as_str)
                    .is_some_and(|c| c != "without_mirrors")
                {
                    continue;
                }
                out.push((day(e.get("date")?)?, e.get("downloads")?.as_i64()?));
            }
        }
        // {"downloads": [{"day": "…", "downloads": n}], "package": …}
        Registry::Npm => {
            for e in v.get("downloads")?.as_array()? {
                out.push((day(e.get("day")?)?, e.get("downloads")?.as_i64()?));
            }
        }
        Registry::Conda => return None,
    }
    Some(out)
}

/// One name, its registry, and what came back.
#[derive(Debug, Clone, PartialEq)]
pub struct Checked {
    pub registry: Registry,
    pub name: String,
    pub answer: Answer,
}

/// What one pass did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pass {
    pub checked: Vec<Checked>,
    pub problems: Vec<Problem>,
    /// Names left for the next run: past [`PER_RUN`], the budget, or a
    /// registry that refused.
    pub deferred: usize,
}

/// The names the registries are asked about this run: the mapping of every
/// selected repository, counted registries only, never a name detected in a
/// private repository — the longest unchecked first.
pub fn due(db: &Path) -> Result<Vec<(Registry, String)>> {
    let conn = Connection::open_with_flags(
        db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let mut stmt = conn.prepare(
        "SELECT p.registry, p.name, min(k.at) AS checked FROM packages p
           JOIN inventory i ON i.repo = p.repo AND i.selected = 1
           LEFT JOIN registry_checks k ON k.registry = p.registry AND k.name = p.name
         WHERE p.name IS NOT NULL AND p.registry IN ('pypi', 'npm')
           AND (i.private = 0 OR p.source = 'owner')
         GROUP BY p.registry, p.name
         ORDER BY checked IS NOT NULL, checked, p.registry, p.name",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut out = Vec::new();
    for row in rows {
        let (registry, name) = row?;
        if let Some(reg) = Registry::parse(&registry) {
            out.push((reg, name));
        }
    }
    Ok(out)
}

/// Ask each registry about each name, at most [`PER_RUN`] within `budget`.
pub async fn run(fetch: &dyn Fetch, todo: &[(Registry, String)], budget: Duration) -> Pass {
    let started = Instant::now();
    let mut pass = Pass::default();
    let mut refused: BTreeSet<Registry> = BTreeSet::new();
    let mut asked = 0;
    for (i, (registry, name)) in todo.iter().enumerate() {
        if asked >= PER_RUN || started.elapsed() >= budget {
            pass.deferred += todo.len() - i;
            if started.elapsed() >= budget {
                pass.problems.push(Problem {
                    kind: Kind::RegistryFailed,
                    step: Step::Registry,
                    repo: None,
                    detail: format!(
                        "the pass's {} s budget was spent; {} package(s) wait for the next run",
                        budget.as_secs(),
                        todo.len() - i
                    ),
                });
            }
            break;
        }
        if refused.contains(registry) {
            pass.deferred += 1;
            continue;
        }
        let Some(url) = url(*registry, name) else {
            continue;
        };
        asked += 1;
        let left = budget.saturating_sub(started.elapsed()).min(CALL_TIMEOUT);
        let answer = match tokio::time::timeout(left, fetch.get(&url)).await {
            Ok(Ok((status, body))) => classify(*registry, status, &body),
            Ok(Err(why)) => Answer::Failed(why),
            Err(_) => Answer::Failed("no answer in time".into()),
        };
        match &answer {
            Answer::Refused(why) => {
                refused.insert(*registry);
                pass.problems.push(Problem {
                    kind: Kind::RegistryRefused,
                    step: Step::Registry,
                    repo: None,
                    detail: format!(
                        "{} refused the collector ({why}); its remaining packages wait for the \
                         next run",
                        registry.as_str()
                    ),
                });
            }
            Answer::Failed(why) => pass.problems.push(Problem {
                kind: Kind::RegistryFailed,
                step: Step::Registry,
                repo: None,
                detail: format!("{} {name}: {why}", registry.as_str()),
            }),
            Answer::Counted(_) | Answer::NotFound => {}
        }
        pass.checked.push(Checked {
            registry: *registry,
            name: name.clone(),
            answer,
        });
    }
    pass
}

/// Write one pass under the run it followed: each day's count (the newest
/// answer's — a registry revises its recent days), each name's last answer,
/// and the problems, in one transaction.
pub fn record_pass(db: &Path, run: DateTime<Utc>, pass: &Pass) -> Result<()> {
    let mut conn = open_store(db)?;
    let tx = conn.transaction()?;
    let run_id = ts(run);
    let at = ts(Utc::now());
    for c in &pass.checked {
        if let Answer::Counted(days) = &c.answer {
            for (day, n) in days {
                tx.execute(
                    "INSERT INTO registry_daily (registry, name, day, downloads, at)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT (registry, name, day) DO UPDATE SET
                       downloads = excluded.downloads, at = excluded.at",
                    params![
                        c.registry.as_str(),
                        c.name,
                        day.format("%Y-%m-%d").to_string(),
                        n,
                        at
                    ],
                )?;
            }
        }
        tx.execute(
            "INSERT INTO registry_checks (registry, name, status, at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (registry, name) DO UPDATE SET status = excluded.status, at = excluded.at",
            params![c.registry.as_str(), c.name, c.answer.status(), at],
        )?;
    }
    for p in &pass.problems {
        tx.execute(
            "INSERT INTO collect_log (run, at, kind, step, repo, detail) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![run_id, at, p.kind.as_str(), p.step.as_str(), p.repo, p.detail],
        )?;
    }
    tx.commit()?;
    Ok(())
}
