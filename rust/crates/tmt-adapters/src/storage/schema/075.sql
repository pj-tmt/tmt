-- A session-learning diagnostic says what its step stopped on as a fixed token (#5016):
-- the decoder family and hook event that refused a payload, or that the driver found no
-- provider process among the hook's ancestors. Never a payload body, path or identifier.
-- The list is `SessionLearningDetail::ALL`; the Rust decoder rejects any other value.
ALTER TABLE launch_observations ADD COLUMN detail TEXT CHECK (detail IS NULL OR detail IN (
    'refused:lifecycle:SessionStart',
    'refused:lifecycle:SessionEnd',
    'refused:prompt:UserPromptSubmit',
    'refused:turn:Stop',
    'refused:turn:Interrupt',
    'refused:activity_session:Notification',
    'refused:activity_session:PostToolUse',
    'refused:activity_session:PostToolUseFailure',
    'refused:unknown',
    'host_unsupported'
));
DROP TRIGGER launch_observations_advances_change_cursor_on_update;
CREATE TRIGGER launch_observations_advances_change_cursor_on_update
AFTER UPDATE ON launch_observations
WHEN OLD.binding_id IS NOT NEW.binding_id
 OR OLD.preferences_fingerprint IS NOT NEW.preferences_fingerprint
 OR OLD.owner_pid IS NOT NEW.owner_pid
 OR OLD.owner_start IS NOT NEW.owner_start
 OR OLD.driver IS NOT NEW.driver
 OR OLD.created_at_ms IS NOT NEW.created_at_ms
 OR OLD.expires_at_ms IS NOT NEW.expires_at_ms
 OR OLD.status IS NOT NEW.status
 OR OLD.provider_pid IS NOT NEW.provider_pid
 OR OLD.provider_transition IS NOT NEW.provider_transition
 OR OLD.provider_start IS NOT NEW.provider_start
 OR OLD.provider_session IS NOT NEW.provider_session
 OR OLD.runtime_mode IS NOT NEW.runtime_mode
 OR OLD.driver_state IS NOT NEW.driver_state
 OR OLD.driver_state_version IS NOT NEW.driver_state_version
 OR OLD.observed_at_ms IS NOT NEW.observed_at_ms
 OR OLD.step IS NOT NEW.step
 OR OLD.reason IS NOT NEW.reason
 OR OLD.detail IS NOT NEW.detail
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
