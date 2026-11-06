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
        tx.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS doomed (id TEXT PRIMARY KEY); DELETE FROM doomed;",
        )?;
        tx.execute(
            "INSERT INTO doomed SELECT object_id FROM source_item WHERE digest=?1",
            [&digest],
        )?;
        loop {
            let added=tx.execute("INSERT OR IGNORE INTO doomed SELECT r.object_id FROM dependency d JOIN revision i ON i.id=d.input JOIN revision r ON r.id=d.output WHERE i.object_id IN (SELECT id FROM doomed)",[])?;
            let lifecycle_added =
                durable::extend_forgotten(&tx)? + durable_decision::extend_forgotten(&tx)?;
            if added + lifecycle_added == 0 {
                break;
            }
        }
        durable_decision::invalidate_forgotten(&tx)?;
        durable::invalidate_forgotten(&tx)?;
        let objects = usize::try_from(
            tx.query_row("SELECT count(*) FROM doomed", [], |r| r.get::<_, i64>(0))?,
        )?;
        let revisions = usize::try_from(tx.query_row(
            "SELECT count(*) FROM revision WHERE object_id IN (SELECT id FROM doomed)",
            [],
            |r| r.get::<_, i64>(0),
        )?)?;
        tx.execute(
            "INSERT OR IGNORE INTO restriction SELECT id FROM doomed",
            [],
        )?;
        // Persist restrictions and erasure atomically; no model jobs run in this slice.
        tx.execute(
            "UPDATE meta SET security_epoch=security_epoch+1 WHERE singleton=1",
            [],
        )?;
        let epoch: i64 = tx.query_row("SELECT security_epoch FROM meta", [], |r| r.get(0))?;
        tx.execute_batch("DELETE FROM memory_fts WHERE revision IN (SELECT id FROM revision WHERE object_id IN (SELECT id FROM doomed));
          DELETE FROM scope WHERE revision IN (SELECT id FROM revision WHERE object_id IN (SELECT id FROM doomed));
          DELETE FROM dependency WHERE output IN (SELECT id FROM revision WHERE object_id IN (SELECT id FROM doomed)) OR input IN (SELECT id FROM revision WHERE object_id IN (SELECT id FROM doomed));
          UPDATE revision SET previous=NULL WHERE object_id IN (SELECT id FROM doomed);
          DELETE FROM revision WHERE object_id IN (SELECT id FROM doomed);
          DELETE FROM source_item WHERE object_id IN (SELECT id FROM doomed);
          DELETE FROM doomed;
          INSERT INTO memory_fts(memory_fts) VALUES ('rebuild');")?;
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
