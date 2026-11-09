//! Logical relational inventory. Hashes stay host-only and never enter reports.
use anyhow::{ensure, Result};
use rusqlite::{types::ValueRef, Connection};
use sha2::{Digest, Sha256};

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Inventory {
    schema: [u8; 32],
    tables: Vec<Table>,
    application: i64,
    version: i64,
}
#[derive(Debug, PartialEq, Eq)]
struct Table {
    name: String,
    columns: [u8; 32],
    rows: [u8; 32],
    count: u64,
}
impl Inventory {
    pub(super) fn counts(&self) -> (usize, u64) {
        (self.tables.len(), self.tables.iter().map(|t| t.count).sum())
    }
}
fn rows(conn: &Connection, sql: &str, params: &[&dyn rusqlite::ToSql]) -> Result<([u8; 32], u64)> {
    let mut stmt = conn.prepare(sql)?;
    let columns = stmt.column_count();
    let mut query = stmt.query(params)?;
    let mut hashes = Vec::<[u8; 32]>::new();
    while let Some(row) = query.next()? {
        let mut h = Sha256::new();
        h.update((columns as u64).to_be_bytes());
        for i in 0..columns {
            match row.get_ref(i)? {
                ValueRef::Null => h.update([0]),
                ValueRef::Integer(v) => {
                    h.update([1]);
                    h.update(v.to_be_bytes());
                }
                ValueRef::Real(v) => {
                    h.update([2]);
                    h.update(v.to_bits().to_be_bytes());
                }
                ValueRef::Text(v) => {
                    h.update([3]);
                    h.update((v.len() as u64).to_be_bytes());
                    h.update(v);
                }
                ValueRef::Blob(v) => {
                    h.update([4]);
                    h.update((v.len() as u64).to_be_bytes());
                    h.update(v);
                }
            }
        }
        hashes.push(h.finalize().into());
    }
    hashes.sort_unstable();
    let count = hashes.len() as u64;
    let mut h = Sha256::new();
    h.update(count.to_be_bytes());
    for hash in hashes {
        h.update(hash);
    }
    Ok((h.finalize().into(), count))
}
pub(super) fn inventory(conn: &Connection) -> Result<Inventory> {
    let schema = rows(
        conn,
        "SELECT type,name,tbl_name,sql FROM sqlite_schema",
        &[],
    )?
    .0;
    let names = conn
        .prepare("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut tables = Vec::new();
    for name in names {
        let columns = rows(conn, "SELECT * FROM pragma_table_xinfo(?1)", &[&name])?.0;
        let quoted = name.replace('"', "\"\"");
        // Canonical identities are declared columns; FTS row identity lives in
        // its copied content/docsize shadow tables. Allocation rowids elsewhere
        // are not application identifiers.
        let (hash, count) = rows(conn, &format!("SELECT * FROM \"{quoted}\""), &[])?;
        tables.push(Table {
            name,
            columns,
            rows: hash,
            count,
        });
    }
    Ok(Inventory {
        schema,
        tables,
        application: conn.query_row("PRAGMA application_id", [], |r| r.get(0))?,
        version: conn.query_row("PRAGMA user_version", [], |r| r.get(0))?,
    })
}
pub(super) fn integrity(conn: &Connection) -> Result<()> {
    let mut cipher = conn.prepare("PRAGMA cipher_integrity_check")?;
    ensure!(
        cipher.query([])?.next()?.is_none(),
        "encrypted snapshot page authentication failed"
    );
    let checks = conn
        .prepare("PRAGMA integrity_check")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    ensure!(checks == ["ok"], "snapshot integrity check failed");
    ensure!(
        conn.prepare("PRAGMA foreign_key_check")?
            .query([])?
            .next()?
            .is_none(),
        "snapshot foreign-key check failed"
    );
    super::super::sql::check_identity(conn)?;
    ensure!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? == 8,
        "snapshots require schema v8"
    );
    Ok(())
}
