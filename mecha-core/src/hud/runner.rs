//! The SQLite runner: one model-drafted query against one owner-registered
//! file, confined to reading that file.
//!
//! A read-only open bounds writes, not reach. `ATTACH DATABASE '<any path>'`
//! is a read-only statement that would let a drafted query read any SQLite
//! file on the host, and `VACUUM INTO '<path>'` writes a file through a
//! read-only connection — both paths from tool input that never met
//! `ToolCtx::resolve` (design §3.1). So the connection is confined four ways,
//! each standing on its own:
//!
//! - **no attached databases** — `SQLITE_LIMIT_ATTACHED` is 0;
//! - **an allowlist authorizer** — `SELECT`, `READ`, `FUNCTION` and
//!   `RECURSIVE` are allowed and every other action is denied, which needs no
//!   knowledge of how a given statement is implemented (`VACUUM INTO` runs as
//!   an `ATTACH`; this is what refuses it);
//! - **functions by name** — `load_extension()`, `fts3_tokenizer()` and
//!   `sqlcipher_export()` arrive as `FUNCTION` and are refused by name;
//!   the load-extension switch is also off, as SQLite ships it, and nothing
//!   here turns it on;
//! - **one statement** — `prepare` refuses a tail, and the prepared statement
//!   must report itself read-only.
//!
//! The row cap and the byte budget are enforced *while fetching*, so a query
//! that would return a million rows is stopped at `max_rows + 1`, never
//! materialised first. A wall-clock budget interrupts a query that runs too
//! long. No error here echoes a value from the result.

use std::path::Path;
use std::time::{Duration, Instant};

use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::limits::Limit;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use super::loader::MAX_DATASET_BYTES;

/// How long one refresh's query may run before it is interrupted.
pub const QUERY_TIMEOUT: Duration = Duration::from_secs(60);

/// What a query returned: its column names in order, and the rows.
#[derive(Debug, Clone, PartialEq)]
pub struct Fetched {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

/// Why a query did not produce rows. Never carries a value from the result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    /// The file could not be opened read-only.
    Open(String),
    /// SQLite failed the statement — a syntax error, a missing table, a
    /// locked file. SQLite's own message, which names the query, never the
    /// data. Classified as a failure rather than a refusal: the message does
    /// not say whether the loader or the source moved, and either way the
    /// previous dataset stays and the doctor reports it as broken.
    Sql(String),
    /// More than one statement.
    MultipleStatements,
    /// The confinement refused an action the statement needs — `ATTACH`,
    /// `PRAGMA`, a write, a refused function. The loader's fault, not the
    /// environment's.
    NotAllowed(String),
    /// The statement would write.
    NotReadOnly,
    /// A value of a type a dataset cannot hold (a BLOB, a non-finite real,
    /// text that is not UTF-8).
    Unrepresentable {
        row: usize,
        column: String,
    },
    TooManyRows {
        max: u32,
    },
    TooLarge {
        max_bytes: usize,
    },
    /// Ran past the wall-clock budget. Classified as a failure: a slow query
    /// may be the loader's (accidentally quadratic) or the source's (grown,
    /// locked), and the message cannot tell which.
    Timeout,
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Open(why) => write!(f, "the source could not be opened read-only: {why}"),
            RunError::Sql(why) => write!(f, "the query failed: {why}"),
            RunError::MultipleStatements => write!(f, "a loader runs exactly one statement"),
            RunError::NotAllowed(why) => write!(
                f,
                "the statement needs an action a loader may not take (only reads are allowed): {why}"
            ),
            RunError::NotReadOnly => write!(f, "the statement would write; a loader only reads"),
            RunError::Unrepresentable { row, column } => {
                // A result column's name may come from a schema someone else
                // wrote (`SELECT *`); show it only when it is a plain
                // identifier, as `ShapeRefusal::Columns` does.
                let column = if super::is_identifier(column) {
                    column.as_str()
                } else {
                    "(unnamed)"
                };
                write!(
                    f,
                    "row {row}, column {column:?}: a blob, a non-finite number or non-UTF-8 text, \
                     which a dataset cannot hold"
                )
            }
            RunError::TooManyRows { max } => {
                write!(f, "the query returned more than max_rows ({max}) rows")
            }
            RunError::TooLarge { max_bytes } => write!(
                f,
                "the result is larger than {max_bytes} bytes; aggregate in the query"
            ),
            RunError::Timeout => write!(
                f,
                "the query ran longer than {}s and was interrupted",
                QUERY_TIMEOUT.as_secs()
            ),
        }
    }
}

impl std::error::Error for RunError {}

/// Functions the allowlist would admit as `FUNCTION` and must not.
const REFUSED_FUNCTIONS: &[&str] = &["load_extension", "fts3_tokenizer", "sqlcipher_export"];

/// Run `query` against the SQLite file at `path`, confined (module docs).
pub fn run_sqlite(
    path: &Path,
    query: &str,
    max_rows: u32,
    timeout: Duration,
) -> Result<Fetched, RunError> {
    let mut conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| RunError::Open(e.to_string()))?;
    confine(&mut conn).map_err(|e| RunError::Open(e.to_string()))?;
    // A writer mid-commit (the host sampler) makes a reader wait rather than
    // fail the refresh: the timers are offset, and this is the backstop for
    // when they are not. rusqlite already defaults to five seconds; set here
    // so the guarantee does not rest on that default.
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|e| RunError::Open(e.to_string()))?;

    let started = Instant::now();
    let timed_out = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = timed_out.clone();
    // Since rusqlite 0.40 installing a handler can fail; a query that would
    // run with no timeout is refused rather than run unbounded.
    conn.progress_handler(
        10_000,
        Some(move || {
            let over = started.elapsed() > timeout;
            if over {
                flag.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            over
        }),
    )
    .map_err(|e| RunError::Open(e.to_string()))?;
    let sql_error = |e: rusqlite::Error| {
        if timed_out.load(std::sync::atomic::Ordering::Relaxed) {
            RunError::Timeout
        } else if matches!(e, rusqlite::Error::MultipleStatement) {
            RunError::MultipleStatements
        } else if matches!(
            &e,
            rusqlite::Error::SqliteFailure(f, _)
                if f.code == rusqlite::ErrorCode::AuthorizationForStatementDenied
        ) {
            RunError::NotAllowed(e.to_string())
        } else {
            RunError::Sql(e.to_string())
        }
    };

    let mut stmt = conn.prepare(query).map_err(sql_error)?;
    if !stmt.readonly() {
        return Err(RunError::NotReadOnly);
    }
    let columns: Vec<String> = stmt.column_names().map(|c| c.to_string()).collect();
    let mut rows = stmt.query([]).map_err(sql_error)?;

    let mut out = Vec::new();
    let mut bytes = 0usize;
    while let Some(row) = rows.next().map_err(sql_error)? {
        if out.len() == max_rows as usize {
            return Err(RunError::TooManyRows { max: max_rows });
        }
        let mut values = Vec::with_capacity(columns.len());
        for (c, name) in columns.iter().enumerate() {
            let unrepresentable = || RunError::Unrepresentable {
                row: out.len(),
                column: name.clone(),
            };
            let v = row.get_ref(c).map_err(sql_error)?;
            let (value, size) = match v {
                ValueRef::Null => (Value::Null, 4),
                ValueRef::Integer(i) => (Value::from(i), 20),
                ValueRef::Real(f) => (
                    serde_json::Number::from_f64(f)
                        .map(Value::Number)
                        .ok_or_else(unrepresentable)?,
                    24,
                ),
                ValueRef::Text(t) => {
                    let s = std::str::from_utf8(t).map_err(|_| unrepresentable())?;
                    (Value::String(s.to_string()), s.len() + 2)
                }
                ValueRef::Blob(_) => return Err(unrepresentable()),
            };
            bytes += size;
            if bytes > MAX_DATASET_BYTES {
                return Err(RunError::TooLarge {
                    max_bytes: MAX_DATASET_BYTES,
                });
            }
            values.push(value);
        }
        out.push(values);
    }
    Ok(Fetched { columns, rows: out })
}

/// The limit on its own, so a test can show it stands without the authorizer
/// (which refuses `ATTACH` earlier, at prepare time) — the two are redundant
/// on purpose, and each is measured alone.
fn no_attached_databases(conn: &Connection) -> rusqlite::Result<()> {
    conn.set_limit(Limit::SQLITE_LIMIT_ATTACHED, 0)?;
    Ok(())
}

/// The four confinements, applied before any statement is prepared.
fn confine(conn: &mut Connection) -> rusqlite::Result<()> {
    no_attached_databases(conn)?;
    conn.authorizer(Some(|ctx: AuthContext<'_>| match ctx.action {
        AuthAction::Select | AuthAction::Read { .. } | AuthAction::Recursive => {
            Authorization::Allow
        }
        AuthAction::Function { function_name }
            if !REFUSED_FUNCTIONS
                .iter()
                .any(|f| f.eq_ignore_ascii_case(function_name)) =>
        {
            Authorization::Allow
        }
        _ => Authorization::Deny,
    }))?;
    Ok(())
}

#[cfg(test)]
mod layers {
    use super::*;

    /// With no authorizer at all, the limit alone refuses an `ATTACH` — so
    /// deleting either confinement fails a test.
    #[test]
    fn the_attach_limit_stands_without_the_authorizer() {
        let dir = std::env::temp_dir().join(format!("mecha-hud-limit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let other = dir.join("other.sqlite");
        Connection::open(&other)
            .unwrap()
            .execute_batch("CREATE TABLE t(x)")
            .unwrap();
        let conn = Connection::open_in_memory().unwrap();
        no_attached_databases(&conn).unwrap();
        let err = conn
            .execute(&format!("ATTACH DATABASE '{}' AS o", other.display()), [])
            .unwrap_err()
            .to_string();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(err.contains("too many attached databases"), "{err}");
    }
}

#[cfg(test)]
#[test]
fn cipher_export_is_refused_by_the_authorizer_at_preparation() {
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch("CREATE VIEW export_probe AS SELECT sqlcipher_export('other');")
        .unwrap();
    let queries = [
        "SELECT SqLcIpHeR_ExPoRt('other')",
        "SELECT sqlcipher_export('other', 'main')",
        "SELECT (SELECT sqlcipher_export('other'))",
        "SELECT * FROM export_probe",
    ];
    for sql in queries {
        assert!(
            connection.prepare(sql).is_ok(),
            "export builtin unavailable for {sql}"
        );
    }
    confine(&mut connection).unwrap();
    for sql in queries {
        let error = connection
            .prepare(sql)
            .expect_err("export must fail at prepare");
        assert!(
            error.to_string().contains("not authorized to use function"),
            "{error}"
        );
    }
}

#[cfg(test)]
#[test]
fn cipher_pragmas_are_refused_without_mutating_global_defaults_or_files() {
    use std::{fs, process::Command};

    const PROBE_DIR: &str = "MECHA_HUD_CIPHER_PRAGMA_PROBE_DIR";
    let Some(root) = std::env::var_os(PROBE_DIR).map(std::path::PathBuf::from) else {
        // SQLCipher settings can act during preparation and are process-wide.
        // A regressing authorizer must not alter concurrent tests.
        let root = std::env::temp_dir().join(format!("mecha-hud-cipher-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let test_name = concat!(
            module_path!(),
            "::cipher_pragmas_are_refused_without_mutating_global_defaults_or_files"
        );
        let (_, name) = test_name.split_once("::").unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--nocapture"])
            .env(PROBE_DIR, &root)
            .output()
            .unwrap();
        fs::remove_dir_all(root).unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    };

    let log = root.join("cipher.log");
    let db = root.join("plain.sqlite");
    let trusted = Connection::open(&db).unwrap();
    trusted
        .execute_batch("CREATE TABLE t(x); INSERT INTO t VALUES (7);")
        .unwrap();
    let read_defaults = || {
        [
            "cipher_default_kdf_iter",
            "cipher_default_use_hmac",
            "cipher_default_plaintext_header_size",
            "cipher_log_level",
        ]
        .map(|name| {
            // SQLCipher's PRAGMAs are not listed by SQLite's pragma_list;
            // query fixed statements directly.
            trusted
                .query_row(&format!("PRAGMA {name}"), [], |r| {
                    r.get::<_, rusqlite::types::Value>(0)
                })
                .unwrap()
        })
    };
    let defaults = read_defaults();
    let before = fs::read(&db).unwrap();
    let quoted_log = log.to_string_lossy().replace('\'', "''");
    let log_pragma = format!("PRAGMA cipher_log = '{quoted_log}'");
    for query in [
        log_pragma.as_str(),
        "PRAGMA cipher_log_level = 'none'",
        "PRAGMA cipher_default_kdf_iter = 12345",
        "PRAGMA cipher_default_use_hmac = OFF",
        "PRAGMA cipher_default_plaintext_header_size = 32",
        "PRAGMA cipher_migrate",
        "PRAGMA key = 'authored-fixture'",
        "PRAGMA rekey = 'authored-fixture'",
    ] {
        assert!(
            matches!(
                run_sqlite(&db, query, 1, Duration::from_secs(1)),
                Err(RunError::NotAllowed(_))
            ),
            "{query}"
        );
    }
    // Optional table-valued access must not succeed.
    assert!(run_sqlite(
        &db,
        "SELECT * FROM pragma_cipher_default_use_hmac",
        1,
        Duration::from_secs(1)
    )
    .is_err());
    assert!(!log.exists());
    assert_eq!(fs::read(&db).unwrap(), before);
    assert_eq!(read_defaults(), defaults);
}
