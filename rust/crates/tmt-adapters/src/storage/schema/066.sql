-- A final can answer a remote request (#4536): request_responses.route_kind gains 'remote'. A
-- remote final has no pane endpoint, no inbox recipient and no host; its route lives on its
-- attempt row, which one attempt_id read reaches. SQLite cannot alter a CHECK, so the table is
-- rebuilt as SQLite's table-redefinition procedure prescribes; nothing references
-- request_responses, and the runner applies this migration with foreign keys off. Every column,
-- index and change-cursor trigger is recreated as it was.
CREATE TABLE request_responses_v66 (
  request_id TEXT PRIMARY KEY,
  attempt_id TEXT NOT NULL,
  route_kind TEXT NOT NULL DEFAULT 'pane' CHECK (route_kind IN ('pane', 'inbox', 'remote')),
  route_recipient_identity_id TEXT,
  server_id TEXT,
  socket_path TEXT,
  server_pid INTEGER CHECK (server_pid IS NULL OR server_pid > 0),
  server_start_time TEXT,
  pane_id TEXT,
  pane_pid INTEGER CHECK (pane_pid IS NULL OR pane_pid > 0),
  body TEXT NOT NULL,
  body_bytes INTEGER NOT NULL CHECK (body_bytes >= 0),
  submitted_at_ms INTEGER NOT NULL CHECK (submitted_at_ms > 0),
  response_expires_at_ms INTEGER NOT NULL DEFAULT 0, host TEXT
  CHECK (host IS NULL OR (length(host) BETWEEN 1 AND 32
    AND host GLOB '[a-z]*' AND host NOT GLOB '*[^a-z0-9-]*')),
  CHECK (
    (route_kind = 'pane' AND route_recipient_identity_id IS NULL AND
      server_id IS NOT NULL AND socket_path IS NOT NULL AND server_pid IS NOT NULL AND
      server_start_time IS NOT NULL AND pane_id IS NOT NULL AND pane_pid IS NOT NULL) OR
    (route_kind = 'inbox' AND route_recipient_identity_id IS NOT NULL AND
      server_id IS NULL AND socket_path IS NULL AND server_pid IS NULL AND
      server_start_time IS NULL AND pane_id IS NULL AND pane_pid IS NULL) OR
    (route_kind = 'remote' AND route_recipient_identity_id IS NULL AND
      server_id IS NULL AND socket_path IS NULL AND server_pid IS NULL AND
      server_start_time IS NULL AND pane_id IS NULL AND pane_pid IS NULL AND host IS NULL)
  )
);

INSERT INTO request_responses_v66 SELECT * FROM request_responses;

DROP TABLE request_responses;
ALTER TABLE request_responses_v66 RENAME TO request_responses;

CREATE INDEX request_responses_attempt ON request_responses (attempt_id, response_expires_at_ms);
CREATE INDEX request_responses_expiry ON request_responses (response_expires_at_ms, request_id);
CREATE INDEX request_responses_retention ON request_responses (submitted_at_ms);

CREATE TRIGGER request_responses_advances_change_cursor_on_insert AFTER INSERT ON request_responses
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_responses_advances_change_cursor_on_delete AFTER DELETE ON request_responses
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_responses_advances_change_cursor_on_update AFTER UPDATE ON request_responses
WHEN OLD.request_id IS NOT NEW.request_id
   OR OLD.attempt_id IS NOT NEW.attempt_id
   OR OLD.route_kind IS NOT NEW.route_kind
   OR OLD.route_recipient_identity_id IS NOT NEW.route_recipient_identity_id
   OR OLD.server_id IS NOT NEW.server_id
   OR OLD.socket_path IS NOT NEW.socket_path
   OR OLD.server_pid IS NOT NEW.server_pid
   OR OLD.server_start_time IS NOT NEW.server_start_time
   OR OLD.pane_id IS NOT NEW.pane_id
   OR OLD.pane_pid IS NOT NEW.pane_pid
   OR OLD.body IS NOT NEW.body
   OR OLD.body_bytes IS NOT NEW.body_bytes
   OR OLD.submitted_at_ms IS NOT NEW.submitted_at_ms
   OR OLD.response_expires_at_ms IS NOT NEW.response_expires_at_ms
   OR OLD.host IS NOT NEW.host
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
