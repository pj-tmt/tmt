-- A request can have a remote route (#4536): route_kind gains 'remote' with the route's
-- own columns. SQLite cannot alter a CHECK, so request_attempts is rebuilt as SQLite's
-- table-redefinition procedure prescribes. The runner applies this migration with foreign keys
-- off, so dropping the old table cannot cascade into the tables that reference it; the new
-- table takes the name and the children's REFERENCES resolve to it again. Every index and
-- change-cursor trigger is recreated as it was; the update trigger also watches the new columns.
CREATE TABLE request_attempts_v65 (
  attempt_id TEXT PRIMARY KEY,
  request_id TEXT NOT NULL UNIQUE,
  nonce TEXT,
  identity_id TEXT REFERENCES identities(id) ON DELETE SET NULL,
  route_kind TEXT NOT NULL DEFAULT 'pane' CHECK (route_kind IN ('pane', 'inbox', 'remote')),
  server_id TEXT,
  socket_path TEXT,
  server_pid INTEGER CHECK (server_pid IS NULL OR server_pid > 0),
  server_start_time TEXT,
  pane_id TEXT,
  pane_pid INTEGER CHECK (pane_pid IS NULL OR pane_pid > 0),
  wait_active INTEGER NOT NULL CHECK (wait_active IN (0, 1)),
  status TEXT NOT NULL CHECK (
    status IN ('prepared', 'sending', 'sent', 'queued', 'uncertain', 'definitely_failed')
  ),
  preamble_every INTEGER CHECK (preamble_every IS NULL OR preamble_every > 0),
  inject_preamble INTEGER NOT NULL CHECK (inject_preamble IN (0, 1)),
  cadence_reserved INTEGER NOT NULL CHECK (cadence_reserved IN (0, 1)),
  prepared_at_ms INTEGER NOT NULL,
  sending_at_ms INTEGER,
  settled_at_ms INTEGER,
  wait_released_at_ms INTEGER,
  expires_at_ms INTEGER NOT NULL,
  response_submitted_at_ms INTEGER,
  retention_days INTEGER NOT NULL DEFAULT 7,
  retention_expires_at_ms INTEGER NOT NULL DEFAULT 0,
  originator_kind TEXT NOT NULL DEFAULT 'unknown' CHECK (originator_kind IN ('unknown', 'explicit', 'verified')),
  originator_identity_id TEXT CHECK (
    (originator_kind = 'unknown' AND originator_identity_id IS NULL) OR
    (originator_kind IN ('explicit', 'verified') AND originator_identity_id IS NOT NULL)
  ),
  recipient_identity_id TEXT,
  message_text TEXT,
  message_bytes INTEGER CHECK (message_bytes IS NULL OR message_bytes >= 0),
  message_expires_at_ms INTEGER,
  attention_revision INTEGER NOT NULL DEFAULT 0 CHECK (
    typeof(attention_revision) = 'integer' AND
    attention_revision BETWEEN 0 AND 9007199254740991
  ),
  attention_acknowledged_revision INTEGER NOT NULL DEFAULT 0 CHECK (
    typeof(attention_acknowledged_revision) = 'integer' AND
    attention_acknowledged_revision BETWEEN 0 AND attention_revision
  ),
  recipient_attention_revision INTEGER NOT NULL DEFAULT 0 CHECK (
    typeof(recipient_attention_revision) = 'integer' AND
    recipient_attention_revision BETWEEN 0 AND 9007199254740991
  ),
  recipient_attention_acknowledged_revision INTEGER NOT NULL DEFAULT 0 CHECK (
    typeof(recipient_attention_acknowledged_revision) = 'integer' AND
    recipient_attention_acknowledged_revision BETWEEN 0 AND recipient_attention_revision
  ), request_kind TEXT NOT NULL DEFAULT 'request'
    CHECK (request_kind IN ('request', 'announcement'))
    CHECK (request_kind = 'request' OR (
        route_kind = 'inbox' AND wait_active = 0 AND preamble_every IS NULL
        AND response_submitted_at_ms IS NULL
    )), room_id TEXT, wake_state TEXT NOT NULL DEFAULT 'not_attempted'
  CHECK (wake_state IN ('not_attempted', 'claimed', 'sent', 'unavailable', 'uncertain')), host TEXT
  CHECK (host IS NULL OR (route_kind = 'pane' AND length(host) BETWEEN 1 AND 32
    AND host GLOB '[a-z]*' AND host NOT GLOB '*[^a-z0-9-]*')), withdrawn_at_ms INTEGER
    CHECK (withdrawn_at_ms IS NULL OR withdrawn_at_ms BETWEEN 1 AND 9007199254740991)
    CHECK (withdrawn_at_ms IS NULL OR response_submitted_at_ms IS NULL), withdrawal_reason TEXT
    CHECK ((withdrawn_at_ms IS NULL AND withdrawal_reason IS NULL)
        OR (withdrawn_at_ms IS NOT NULL AND withdrawal_reason IS NOT NULL
            AND length(CAST(withdrawal_reason AS BLOB)) BETWEEN 1 AND 1024)), origin TEXT
    CHECK (origin IS NULL OR origin IN ('person', 'automation')),
  remote_ref TEXT
    CHECK (remote_ref IS NULL OR length(CAST(remote_ref AS BLOB)) BETWEEN 1 AND 64),
  remote_target_id TEXT
    CHECK (remote_target_id IS NULL OR remote_target_id GLOB '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]-[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
  remote_target_name TEXT
    CHECK (remote_target_name IS NULL OR length(CAST(remote_target_name AS BLOB)) BETWEEN 1 AND 128),
  remote_sender_name TEXT
    CHECK (remote_sender_name IS NULL OR length(CAST(remote_sender_name AS BLOB)) BETWEEN 1 AND 128),
  CHECK (
    (route_kind = 'pane' AND server_id IS NOT NULL AND socket_path IS NOT NULL AND
      server_pid IS NOT NULL AND server_start_time IS NOT NULL AND pane_id IS NOT NULL AND
      pane_pid IS NOT NULL) OR
    (route_kind = 'inbox' AND recipient_identity_id IS NOT NULL AND
      server_id IS NULL AND socket_path IS NULL AND server_pid IS NULL AND
      server_start_time IS NULL AND pane_id IS NULL AND pane_pid IS NULL) OR
    (route_kind = 'remote' AND originator_kind IN ('explicit', 'verified') AND
      recipient_identity_id IS NULL AND remote_ref IS NOT NULL AND
      remote_target_name IS NOT NULL AND remote_sender_name IS NOT NULL AND
      server_id IS NULL AND socket_path IS NULL AND server_pid IS NULL AND
      server_start_time IS NULL AND pane_id IS NULL AND pane_pid IS NULL)
  ),
  CHECK (route_kind = 'remote' OR (remote_ref IS NULL AND remote_target_id IS NULL AND
    remote_target_name IS NULL AND remote_sender_name IS NULL))
);

INSERT INTO request_attempts_v65 (
  attempt_id, request_id, nonce, identity_id,
  route_kind, server_id, socket_path, server_pid,
  server_start_time, pane_id, pane_pid, wait_active,
  status, preamble_every, inject_preamble, cadence_reserved,
  prepared_at_ms, sending_at_ms, settled_at_ms, wait_released_at_ms,
  expires_at_ms, response_submitted_at_ms, retention_days, retention_expires_at_ms,
  originator_kind, originator_identity_id, recipient_identity_id, message_text,
  message_bytes, message_expires_at_ms, attention_revision, attention_acknowledged_revision,
  recipient_attention_revision, recipient_attention_acknowledged_revision, request_kind, room_id,
  wake_state, host, withdrawn_at_ms, withdrawal_reason,
  origin
)
SELECT
  attempt_id, request_id, nonce, identity_id,
  route_kind, server_id, socket_path, server_pid,
  server_start_time, pane_id, pane_pid, wait_active,
  status, preamble_every, inject_preamble, cadence_reserved,
  prepared_at_ms, sending_at_ms, settled_at_ms, wait_released_at_ms,
  expires_at_ms, response_submitted_at_ms, retention_days, retention_expires_at_ms,
  originator_kind, originator_identity_id, recipient_identity_id, message_text,
  message_bytes, message_expires_at_ms, attention_revision, attention_acknowledged_revision,
  recipient_attention_revision, recipient_attention_acknowledged_revision, request_kind, room_id,
  wake_state, host, withdrawn_at_ms, withdrawal_reason,
  origin
FROM request_attempts;

DROP TABLE request_attempts;
ALTER TABLE request_attempts_v65 RENAME TO request_attempts;

CREATE INDEX request_attempts_attention ON request_attempts (originator_identity_id, attention_revision, request_id);
CREATE INDEX request_attempts_cleanup_expiry ON request_attempts (expires_at_ms, attempt_id)
  WHERE wait_active = 1 OR status IN ('prepared', 'sending');
CREATE INDEX request_attempts_endpoint_active ON request_attempts (
  server_id, socket_path, server_pid, server_start_time, pane_id, pane_pid,
  wait_active, status, prepared_at_ms
) WHERE route_kind = 'pane';
CREATE INDEX request_attempts_expiry ON request_attempts (expires_at_ms, status);
CREATE INDEX request_attempts_prompt_expiry ON request_attempts (message_expires_at_ms, attempt_id)
  WHERE message_text IS NOT NULL;
CREATE INDEX request_attempts_recipient_attention ON request_attempts (
  recipient_identity_id, status, recipient_attention_revision, request_id
) WHERE route_kind = 'inbox';
CREATE INDEX request_attempts_response_attention ON request_attempts (
  originator_identity_id, attention_revision, request_id
) WHERE response_submitted_at_ms IS NOT NULL;
CREATE INDEX request_attempts_retention ON request_attempts (wait_active, status, settled_at_ms);
CREATE INDEX request_attempts_retention_horizon ON request_attempts (retention_expires_at_ms, attempt_id);
CREATE INDEX request_attempts_room_recipient_attention ON request_attempts (
  recipient_identity_id, room_id, status, recipient_attention_revision, request_id
) WHERE route_kind = 'inbox';
CREATE INDEX request_attempts_room_response_attention ON request_attempts (
  originator_identity_id, room_id, attention_revision, request_id
) WHERE response_submitted_at_ms IS NOT NULL;
CREATE INDEX request_history_originator_results
    ON request_attempts(originator_identity_id, response_submitted_at_ms DESC, request_id DESC)
    WHERE response_submitted_at_ms IS NOT NULL;
CREATE INDEX request_history_recipient ON request_attempts
  (recipient_identity_id, prepared_at_ms DESC, request_id DESC);
CREATE INDEX request_history_recipient_room ON request_attempts
  (recipient_identity_id, room_id, prepared_at_ms DESC, request_id DESC);
CREATE INDEX request_history_room ON request_attempts
  (room_id, prepared_at_ms DESC, request_id DESC);

CREATE TRIGGER request_attempts_advances_change_cursor_on_insert AFTER INSERT ON request_attempts
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_attempts_advances_change_cursor_on_delete AFTER DELETE ON request_attempts
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_attempts_advances_change_cursor_on_update AFTER UPDATE ON request_attempts
WHEN OLD.attempt_id IS NOT NEW.attempt_id
   OR OLD.request_id IS NOT NEW.request_id
   OR OLD.nonce IS NOT NEW.nonce
   OR OLD.identity_id IS NOT NEW.identity_id
   OR OLD.route_kind IS NOT NEW.route_kind
   OR OLD.server_id IS NOT NEW.server_id
   OR OLD.socket_path IS NOT NEW.socket_path
   OR OLD.server_pid IS NOT NEW.server_pid
   OR OLD.server_start_time IS NOT NEW.server_start_time
   OR OLD.pane_id IS NOT NEW.pane_id
   OR OLD.pane_pid IS NOT NEW.pane_pid
   OR OLD.wait_active IS NOT NEW.wait_active
   OR OLD.status IS NOT NEW.status
   OR OLD.preamble_every IS NOT NEW.preamble_every
   OR OLD.inject_preamble IS NOT NEW.inject_preamble
   OR OLD.cadence_reserved IS NOT NEW.cadence_reserved
   OR OLD.prepared_at_ms IS NOT NEW.prepared_at_ms
   OR OLD.sending_at_ms IS NOT NEW.sending_at_ms
   OR OLD.settled_at_ms IS NOT NEW.settled_at_ms
   OR OLD.wait_released_at_ms IS NOT NEW.wait_released_at_ms
   OR OLD.expires_at_ms IS NOT NEW.expires_at_ms
   OR OLD.response_submitted_at_ms IS NOT NEW.response_submitted_at_ms
   OR OLD.retention_days IS NOT NEW.retention_days
   OR OLD.retention_expires_at_ms IS NOT NEW.retention_expires_at_ms
   OR OLD.originator_kind IS NOT NEW.originator_kind
   OR OLD.originator_identity_id IS NOT NEW.originator_identity_id
   OR OLD.recipient_identity_id IS NOT NEW.recipient_identity_id
   OR OLD.message_text IS NOT NEW.message_text
   OR OLD.message_bytes IS NOT NEW.message_bytes
   OR OLD.message_expires_at_ms IS NOT NEW.message_expires_at_ms
   OR OLD.attention_revision IS NOT NEW.attention_revision
   OR OLD.attention_acknowledged_revision IS NOT NEW.attention_acknowledged_revision
   OR OLD.recipient_attention_revision IS NOT NEW.recipient_attention_revision
   OR OLD.recipient_attention_acknowledged_revision IS NOT NEW.recipient_attention_acknowledged_revision
   OR OLD.request_kind IS NOT NEW.request_kind
   OR OLD.room_id IS NOT NEW.room_id
   OR OLD.wake_state IS NOT NEW.wake_state
   OR OLD.host IS NOT NEW.host
   OR OLD.withdrawn_at_ms IS NOT NEW.withdrawn_at_ms
   OR OLD.withdrawal_reason IS NOT NEW.withdrawal_reason
   OR OLD.origin IS NOT NEW.origin
   OR OLD.remote_ref IS NOT NEW.remote_ref
   OR OLD.remote_target_id IS NOT NEW.remote_target_id
   OR OLD.remote_target_name IS NOT NEW.remote_target_name
   OR OLD.remote_sender_name IS NOT NEW.remote_sender_name
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
