//! Fixed-query reader for the synthetic durable-task CLI qualification only.
//! This deliberately bypasses runtime APIs so the pilot checks stored evidence.
use anyhow::{ensure, Result};
use rusqlite::{types::ValueRef, Connection, OpenFlags};
use serde_json::{json, Map, Value};

fn rows(db: &Connection, sql: &str) -> Result<Vec<Value>> {
    let mut statement = db.prepare(sql)?;
    let names: Vec<String> = statement.column_names().map(|s| s.to_string()).collect();
    let mut cursor = statement.query([])?;
    let mut output = Vec::new();
    while let Some(row) = cursor.next()? {
        let mut object = Map::new();
        for (index, name) in names.iter().enumerate() {
            let value = match row.get_ref(index)? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(v) => json!(v),
                ValueRef::Real(v) => json!(v),
                ValueRef::Text(v) => json!(std::str::from_utf8(v)?),
                ValueRef::Blob(_) => anyhow::bail!("unexpected fixture blob"),
            };
            object.insert(name.clone(), value);
        }
        output.push(Value::Object(object));
        ensure!(output.len() <= 4096, "fixture reader row bound exceeded");
    }
    Ok(output)
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        args.len() == 2,
        "usage: memory_task_plan_inspect FIXTURE_DB KEY_FILE"
    );
    let key = std::fs::read_to_string(&args[1])?;
    let key = key.trim();
    ensure!(
        key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid fixture key"
    );
    let db = Connection::open_with_flags(&args[0], OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    db.execute_batch(&format!("PRAGMA key=\"x'{key}'\"; PRAGMA query_only=ON;"))?;
    let cipher: String = db.query_row("PRAGMA cipher_version", [], |r| r.get(0))?;
    ensure!(!cipher.is_empty(), "SQLCipher required");
    let domain: String = db.query_row("SELECT domain FROM meta WHERE singleton=1", [], |r| {
        r.get(0)
    })?;
    ensure!(
        domain == "synthetic-demo",
        "reader only admits synthetic-demo fixtures"
    );
    let foreign: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM revision r JOIN object o ON o.id=r.object_id WHERE o.kind='task' AND (json_extract(r.payload,'$.data.account') IS NULL OR json_extract(r.payload,'$.data.account')<>'fixture-task-cli'))",
        [], |r| r.get(0),
    )?;
    ensure!(
        !foreign,
        "reader only admits fixture-task-cli task accounts"
    );
    let version: u32 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let revisions = rows(
        &db,
        "SELECT o.kind,r.id,r.object_id,r.seq,r.previous,r.lifecycle,r.payload FROM revision r JOIN object o ON o.id=r.object_id ORDER BY r.seq,r.id",
    )?;
    let dependencies = rows(
        &db,
        "SELECT output,input FROM dependency ORDER BY output,input",
    )?;
    let plans = if version >= 10 {
        rows(
            &db,
            "SELECT operation_key,receipt_id,task_object,plan_object,before_revision,after_revision,attachment_digest,turn_id,ordinal,state FROM plan_operation ORDER BY turn_id,ordinal",
        )?
    } else {
        Vec::new()
    };
    let tasks = if version >= 9 {
        rows(&db, "SELECT * FROM task_operation ORDER BY operation_key")?
    } else {
        Vec::new()
    };
    let captures = rows(&db, "SELECT * FROM host_capture_receipt ORDER BY rowid")?;
    let fts = rows(
        &db,
        "SELECT revision,text FROM memory_fts ORDER BY revision",
    )?;
    println!(
        "{}",
        json!({"schema_version":version,"revisions":revisions,"dependencies":dependencies,"plans":plans,"tasks":tasks,"captures":captures,"fts":fts})
    );
    Ok(())
}
