-- Frozen schema-14 supplement to schema13.sql, from reviewed M1 source
-- 69d8e746ce891eade823a33d909f6628e517bf36 (merged 06f52c8f).
-- These historical definitions do not use the migration under test.
CREATE TABLE members(
  remote_id TEXT NOT NULL, member_id TEXT NOT NULL,
  role TEXT NOT NULL CHECK(role IN ('owner','user')),
  display_name TEXT NOT NULL,
  sending_enabled INTEGER NOT NULL CHECK(sending_enabled IN (0,1)),
  revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 9007199254740991),
  created_at_ms INTEGER NOT NULL CHECK(created_at_ms BETWEEN 0 AND 9007199254740991),
  revoked_at_ms INTEGER CHECK(revoked_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY(remote_id,member_id));
CREATE TABLE member_machines(
  remote_id TEXT NOT NULL, client_id TEXT NOT NULL REFERENCES grants(client_id),
  member_id TEXT NOT NULL,
  associated_at_ms INTEGER NOT NULL CHECK(associated_at_ms BETWEEN 0 AND 9007199254740991),
  PRIMARY KEY(remote_id,client_id),
  FOREIGN KEY(remote_id,member_id) REFERENCES members(remote_id,member_id));
CREATE INDEX member_machines_member ON member_machines(remote_id,member_id);
INSERT INTO _migrations VALUES(14,'members','2026-10-25');
