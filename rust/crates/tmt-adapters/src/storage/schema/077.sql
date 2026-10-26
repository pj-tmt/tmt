-- A session-learning diagnostic also names why a decoded hook that cannot bind admitted nothing
-- (#5125): `stopped:<decoder>:<event>:<reason>`, or `stopped:unknown:<reason>`. Still a fixed
-- token, never a payload body, path or identifier. The list is `SessionLearningDetail::ALL`; the
-- Rust decoder rejects any other value. SQLite cannot change a CHECK in place, so the column
-- is replaced by one with the wider list and every stored detail is carried over.
DROP TRIGGER launch_observations_advances_change_cursor_on_update;
ALTER TABLE launch_observations ADD COLUMN detail_next TEXT CHECK (detail_next IS NULL OR detail_next IN (
    'refused:lifecycle:SessionStart',
    'refused:lifecycle:SessionEnd',
    'refused:prompt:UserPromptSubmit',
    'refused:turn:Stop',
    'refused:turn:Interrupt',
    'refused:activity_session:Notification',
    'refused:activity_session:PostToolUse',
    'refused:activity_session:PostToolUseFailure',
    'refused:unknown',
    'host_unsupported',
    'stopped:turn:Stop:observer_owns_stop',
    'stopped:turn:Stop:unbound_session',
    'stopped:turn:Stop:not_current',
    'stopped:turn:Interrupt:observer_owns_stop',
    'stopped:turn:Interrupt:unbound_session',
    'stopped:turn:Interrupt:not_current',
    'stopped:prompt:UserPromptSubmit:unbound_session',
    'stopped:prompt:UserPromptSubmit:not_current',
    'stopped:activity_session:Notification:activity_missing',
    'stopped:activity_session:Notification:unbound_session',
    'stopped:activity_session:Notification:not_current',
    'stopped:activity_session:PostToolUse:activity_missing',
    'stopped:activity_session:PostToolUse:not_waiting',
    'stopped:activity_session:PostToolUse:unbound_session',
    'stopped:activity_session:PostToolUse:not_current',
    'stopped:activity_session:PostToolUseFailure:activity_missing',
    'stopped:activity_session:PostToolUseFailure:not_waiting',
    'stopped:activity_session:PostToolUseFailure:unbound_session',
    'stopped:activity_session:PostToolUseFailure:not_current',
    'stopped:unknown:activity_missing',
    'stopped:unknown:not_waiting',
    'stopped:unknown:observer_owns_stop',
    'stopped:unknown:unbound_session',
    'stopped:unknown:not_current'
));
UPDATE launch_observations SET detail_next = detail;
ALTER TABLE launch_observations DROP COLUMN detail;
ALTER TABLE launch_observations RENAME COLUMN detail_next TO detail;
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
