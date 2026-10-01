-- The pane process's incarnation beside its pid (#570): the start token of
-- the ProcessIncarnation core observed for pane_pid when the binding was
-- created or rebound, so a reused pid cannot pass as the same pane. It never
-- comes from a host's or driver's text. Bindings made before schema 42 keep
-- NULL until they rebind: NULL, like a failed observation, is unknown and
-- proves neither loss nor sameness.
ALTER TABLE bindings ADD COLUMN pane_incarnation TEXT
  CHECK (pane_incarnation IS NULL OR (length(pane_incarnation) BETWEEN 1 AND 256
    AND trim(pane_incarnation) <> '' AND pane_incarnation NOT GLOB '*[^ -~]*'));

DROP TRIGGER bindings_advances_change_cursor_on_update;
CREATE TRIGGER bindings_advances_change_cursor_on_update AFTER UPDATE ON bindings
WHEN OLD.id IS NOT NEW.id
   OR OLD.identity_id IS NOT NEW.identity_id
   OR OLD.transport IS NOT NEW.transport
   OR OLD.pane_id IS NOT NEW.pane_id
   OR OLD.server_id IS NOT NEW.server_id
   OR OLD.socket_path IS NOT NEW.socket_path
   OR OLD.server_pid IS NOT NEW.server_pid
   OR OLD.server_start_time IS NOT NEW.server_start_time
   OR OLD.pane_pid IS NOT NEW.pane_pid
   OR OLD.bound_at IS NOT NEW.bound_at
   OR OLD.runtime_state IS NOT NEW.runtime_state
   OR OLD.last_transition IS NOT NEW.last_transition
   OR OLD.runtime_pid IS NOT NEW.runtime_pid
   OR OLD.runtime_start_identity IS NOT NEW.runtime_start_identity
   OR OLD.observed_provider_session_id IS NOT NEW.observed_provider_session_id
   OR OLD.launch_owner_pid IS NOT NEW.launch_owner_pid
   OR OLD.launch_owner_start_identity IS NOT NEW.launch_owner_start_identity
   OR OLD.pane_incarnation IS NOT NEW.pane_incarnation
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
