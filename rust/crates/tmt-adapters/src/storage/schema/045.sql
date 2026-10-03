-- Keep an admitted fresh launch or explicit resume channel/plain choice.
-- NULL preserves the driver default for legacy identities; no lease is reused.
ALTER TABLE identity_session_preferences ADD COLUMN channel INTEGER
  CHECK (channel IS NULL OR (typeof(channel) = 'integer' AND channel IN (0, 1) AND preferred_harness IS NOT NULL));
DROP TRIGGER identity_session_preferences_advances_change_cursor_on_update;
CREATE TRIGGER identity_session_preferences_advances_change_cursor_on_update AFTER UPDATE ON identity_session_preferences
WHEN OLD.channel IS NOT NEW.channel
   OR OLD.identity_id IS NOT NEW.identity_id
   OR OLD.preferred_harness IS NOT NEW.preferred_harness
   OR OLD.remembered_harness IS NOT NEW.remembered_harness
   OR OLD.runtime_mode IS NOT NEW.runtime_mode
   OR OLD.provider_session_id IS NOT NEW.provider_session_id
   OR OLD.driver_state IS NOT NEW.driver_state
   OR OLD.driver_state_version IS NOT NEW.driver_state_version
   OR OLD.stale_at_ms IS NOT NEW.stale_at_ms
   OR OLD.resume_pending_at_ms IS NOT NEW.resume_pending_at_ms
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
