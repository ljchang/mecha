CREATE TABLE task_operation (
  operation_key TEXT PRIMARY KEY CHECK(length(operation_key)=64),
  payload_hash TEXT, receipt_id TEXT UNIQUE, task_object TEXT,
  before_revision TEXT, after_revision TEXT UNIQUE, operation TEXT,
  state TEXT NOT NULL CHECK(state IN ('active','forgotten')),
  CHECK((state='forgotten' AND payload_hash IS NULL AND receipt_id IS NULL
    AND task_object IS NULL AND before_revision IS NULL AND after_revision IS NULL AND operation IS NULL)
    OR (state='active' AND length(payload_hash)=64 AND receipt_id IS NOT NULL
    AND task_object IS NOT NULL AND after_revision IS NOT NULL
    AND operation IN ('create','revise','status')))
);
CREATE TABLE task_store_state (
  singleton INTEGER PRIMARY KEY CHECK(singleton=1),
  disposition TEXT NOT NULL CHECK(disposition IN ('live','historical'))
);
INSERT INTO task_store_state VALUES(1,'live');
PRAGMA user_version=9;
