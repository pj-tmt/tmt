-- An unpublished binding is a reservation, never an active bindings row.
CREATE TABLE binding_reservations (
  target_binding_id TEXT PRIMARY KEY,
  identity_id TEXT NOT NULL UNIQUE REFERENCES identities(id) ON DELETE CASCADE,
  old_binding_id TEXT,
  transport TEXT NOT NULL,
  server_id TEXT NOT NULL,
  socket_path TEXT NOT NULL,
  server_pid INTEGER NOT NULL CHECK (typeof(server_pid) = 'integer' AND server_pid BETWEEN 1 AND 9007199254740991),
  server_start_time TEXT NOT NULL,
  pane_id TEXT NOT NULL,
  pane_pid INTEGER NOT NULL CHECK (typeof(pane_pid) = 'integer' AND pane_pid BETWEEN 1 AND 9007199254740991),
  pane_incarnation TEXT,
  expected_binding_id TEXT,
  publisher_pid INTEGER NOT NULL CHECK (typeof(publisher_pid) = 'integer' AND publisher_pid BETWEEN 1 AND 9007199254740991),
  publisher_start TEXT NOT NULL CHECK (length(CAST(publisher_start AS BLOB)) BETWEEN 1 AND 256),
  created_at_ms INTEGER NOT NULL CHECK (typeof(created_at_ms) = 'integer' AND created_at_ms BETWEEN 0 AND 9007199254740991),
  expires_at_ms INTEGER NOT NULL CHECK (typeof(expires_at_ms) = 'integer' AND expires_at_ms BETWEEN created_at_ms AND 9007199254740991),
  UNIQUE (transport, server_id, pane_id)
);
CREATE TRIGGER binding_reservations_advances_change_cursor_on_insert AFTER INSERT ON binding_reservations
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER binding_reservations_advances_change_cursor_on_update AFTER UPDATE ON binding_reservations
WHEN OLD.target_binding_id IS NOT NEW.target_binding_id OR OLD.identity_id IS NOT NEW.identity_id
  OR OLD.old_binding_id IS NOT NEW.old_binding_id OR OLD.transport IS NOT NEW.transport
  OR OLD.server_id IS NOT NEW.server_id OR OLD.socket_path IS NOT NEW.socket_path
  OR OLD.server_pid IS NOT NEW.server_pid OR OLD.server_start_time IS NOT NEW.server_start_time
  OR OLD.pane_id IS NOT NEW.pane_id OR OLD.pane_pid IS NOT NEW.pane_pid
  OR OLD.pane_incarnation IS NOT NEW.pane_incarnation OR OLD.expected_binding_id IS NOT NEW.expected_binding_id
  OR OLD.publisher_pid IS NOT NEW.publisher_pid OR OLD.publisher_start IS NOT NEW.publisher_start
  OR OLD.created_at_ms IS NOT NEW.created_at_ms OR OLD.expires_at_ms IS NOT NEW.expires_at_ms
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER binding_reservations_advances_change_cursor_on_delete AFTER DELETE ON binding_reservations
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
