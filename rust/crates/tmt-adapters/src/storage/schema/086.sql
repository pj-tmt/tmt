-- A `tmt run` of a program no driver claims records its bounded basename, never its
-- arguments. Delivery refuses while it is set; the first admitted agent clears it.
ALTER TABLE bindings ADD COLUMN plain_program TEXT
  CHECK (plain_program IS NULL OR length(plain_program) BETWEEN 1 AND 64);
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
   OR OLD.runtime_start_refusal IS NOT NEW.runtime_start_refusal
   OR OLD.plain_program IS NOT NEW.plain_program
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
