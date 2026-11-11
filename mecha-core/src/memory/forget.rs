use super::{sql::source_digest, *};
use anyhow::Result;
use rusqlite::TransactionBehavior;

impl Memory {
    /// Purges this prototype's SQLite content, not backups, prior outputs or media.
    /// Conservative object closure also purges older revisions of affected claims.
    pub fn forget(&mut self, source: &SourceKey) -> Result<ForgetReport> {
        let digest = source_digest(&self.instance, source)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("INSERT OR IGNORE INTO suppression VALUES (?1)", [&digest])?;
        initialize_doomed(&tx)?;
        tx.execute(
            "INSERT INTO doomed SELECT object_id FROM source_item WHERE digest=?1",
            [&digest],
        )?;
        let PurgeCounts { objects, revisions } = purge_seeded(&tx)?;
        // Persist restrictions and erasure atomically; no model jobs run in this slice.
        tx.execute(
            "UPDATE meta SET security_epoch=security_epoch+1 WHERE singleton=1",
            [],
        )?;
        let epoch: i64 = tx.query_row("SELECT security_epoch FROM meta", [], |r| r.get(0))?;
        // Keep only opaque restricted object IDs and salted source digests.
        tx.commit()?;
        self.conn.execute_batch("VACUUM;")?;
        Ok(ForgetReport {
            objects,
            revisions,
            security_epoch: epoch,
            scope: "local prototype database only",
        })
    }
}

pub(super) struct PurgeCounts {
    pub(super) objects: usize,
    pub(super) revisions: usize,
}
pub(super) fn initialize_doomed(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS doomed (id TEXT PRIMARY KEY); DELETE FROM doomed;",
    )?;
    Ok(())
}
/// Caller owns transaction, seeds, epoch and post-commit scrubbing. Keep all
/// lifecycle hooks: ordinary forgetting also supports the older durable schemas.
pub(super) fn purge_seeded(conn: &rusqlite::Connection) -> Result<PurgeCounts> {
    loop {
        let added=conn.execute("INSERT OR IGNORE INTO doomed SELECT r.object_id FROM dependency d JOIN revision i ON i.id=d.input JOIN revision r ON r.id=d.output WHERE i.object_id IN (SELECT id FROM doomed)",[])?;
        let lifecycle_added = host_capture::extend_forgotten(conn)?
            + durable::extend_forgotten(conn)?
            + durable_decision::extend_forgotten(conn)?
            + durable_action::extend_forgotten(conn)?;
        if added + lifecycle_added == 0 {
            break;
        }
    }
    task::invalidate_forgotten(conn)?;
    interpretation::invalidate_forgotten(conn)?;
    host_capture::invalidate_forgotten(conn)?;
    durable_action::invalidate_forgotten(conn)?;
    durable_decision::invalidate_forgotten(conn)?;
    durable::invalidate_forgotten(conn)?;
    let objects = usize::try_from(
        conn.query_row("SELECT count(*) FROM doomed", [], |r| r.get::<_, i64>(0))?,
    )?;
    let revisions = usize::try_from(conn.query_row(
        "SELECT count(*) FROM revision WHERE object_id IN (SELECT id FROM doomed)",
        [],
        |r| r.get::<_, i64>(0),
    )?)?;
    conn.execute(
        "INSERT OR IGNORE INTO restriction SELECT id FROM doomed",
        [],
    )?;
    conn.execute_batch("DELETE FROM memory_fts WHERE revision IN (SELECT id FROM revision WHERE object_id IN (SELECT id FROM doomed));
          DELETE FROM scope WHERE revision IN (SELECT id FROM revision WHERE object_id IN (SELECT id FROM doomed));
          DELETE FROM dependency WHERE output IN (SELECT id FROM revision WHERE object_id IN (SELECT id FROM doomed)) OR input IN (SELECT id FROM revision WHERE object_id IN (SELECT id FROM doomed));
          UPDATE revision SET previous=NULL WHERE object_id IN (SELECT id FROM doomed);
          DELETE FROM revision WHERE object_id IN (SELECT id FROM doomed);
          DELETE FROM source_item WHERE object_id IN (SELECT id FROM doomed);
          DELETE FROM doomed;
          INSERT INTO memory_fts(memory_fts) VALUES ('rebuild');")?;
    Ok(PurgeCounts { objects, revisions })
}
