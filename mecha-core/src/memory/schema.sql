PRAGMA application_id = 1296912466;
PRAGMA user_version = 1;
CREATE TABLE meta (
  singleton INTEGER PRIMARY KEY CHECK(singleton=1), instance TEXT NOT NULL,
  domain TEXT NOT NULL, security_epoch INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE change_set (seq INTEGER PRIMARY KEY AUTOINCREMENT, committed_at TEXT NOT NULL);
CREATE TABLE object (id TEXT PRIMARY KEY, kind TEXT NOT NULL);
CREATE TABLE revision (
  id TEXT PRIMARY KEY, object_id TEXT NOT NULL REFERENCES object(id),
  seq INTEGER NOT NULL REFERENCES change_set(seq), previous TEXT REFERENCES revision(id),
  lifecycle TEXT NOT NULL CHECK(lifecycle IN ('active','withdrawn')),
  payload TEXT NOT NULL CHECK(json_valid(payload)), UNIQUE(object_id,seq)
);
CREATE INDEX revision_head ON revision(object_id,seq DESC);
CREATE TABLE dependency (
  output TEXT NOT NULL REFERENCES revision(id), input TEXT NOT NULL REFERENCES revision(id),
  PRIMARY KEY(output,input), CHECK(output<>input)
);
CREATE INDEX reverse_dependency ON dependency(input,output);
CREATE TABLE scope (revision TEXT NOT NULL REFERENCES revision(id), name TEXT NOT NULL, PRIMARY KEY(revision,name));
CREATE TABLE source_item (digest TEXT PRIMARY KEY, object_id TEXT NOT NULL UNIQUE REFERENCES object(id));
CREATE TABLE suppression (digest TEXT PRIMARY KEY);
CREATE TABLE restriction (object_id TEXT PRIMARY KEY REFERENCES object(id));
CREATE VIRTUAL TABLE memory_fts USING fts5(revision UNINDEXED, text, tokenize='porter unicode61');
INSERT INTO memory_fts(memory_fts, rank) VALUES('secure-delete', 1);
