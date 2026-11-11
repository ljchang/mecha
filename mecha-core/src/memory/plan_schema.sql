CREATE TABLE plan_operation (
 operation_key TEXT PRIMARY KEY CHECK(length(operation_key)=64),
 payload_hash TEXT, receipt_id TEXT UNIQUE, task_object TEXT, plan_object TEXT,
 before_revision TEXT, after_revision TEXT UNIQUE, attachment_digest TEXT,
 turn_id TEXT, permit_digest TEXT, ordinal INTEGER,
 state TEXT NOT NULL CHECK(state IN ('pending','audited','forgotten')),
 CHECK((state='forgotten' AND payload_hash IS NULL AND receipt_id IS NULL
  AND task_object IS NULL AND plan_object IS NULL AND before_revision IS NULL
  AND after_revision IS NULL AND attachment_digest IS NULL AND turn_id IS NULL
  AND permit_digest IS NULL AND ordinal IS NULL)
 OR (state IN ('pending','audited') AND length(payload_hash)=64 AND receipt_id IS NOT NULL
  AND task_object IS NOT NULL AND plan_object IS NOT NULL AND after_revision IS NOT NULL
  AND length(attachment_digest)=64 AND turn_id IS NOT NULL AND length(permit_digest)=64 AND ordinal>=0))
);
PRAGMA user_version=10;
