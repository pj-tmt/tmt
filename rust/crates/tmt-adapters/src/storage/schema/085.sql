-- Preserve bounded Digest worker/caller failure stages, never arbitrary error text.
-- Widen the existing CHECK while preserving stored details and change-cursor triggers.
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
    'stopped:turn:Stop:no_admitted_start',
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
    'stopped:unknown:not_current',
    'digest_stop:worker_spawn',
    'digest_stop:worker_timeout',
    'digest_stop:worker_error',
    'digest_stop:worker_stderr',
    'digest_stop:capture_failed',
    'digest_stop:launch:argument',
    'digest_stop:launch:host',
    'digest_stop:launch:caller',
    'digest_stop:launch:caller_probe',
    'digest_stop:launch:binding',
    'digest_stop:launch:process',
    'digest_stop:launch:session',
    'digest_stop:launch:remembered',
    'digest_stop:launch:endpoint',
    'digest_stop:launch:owner',
    'digest_stop:launch:owner_probe',
    'digest_stop:launch:owner_not_live',
    'digest_stop:launch:deadline',
    'digest_stop:notice_unavailable',
    'digest_stop:handoff_invalid',
    'digest_stop:checklist_changed',
    'digest_stop:worker_exit',
    'digest_stop:worker_signal',
    'digest_stop:worker_io',
    'digest_stop:worker_output_limit',
    'digest_stop:worker:config',
    'digest_stop:worker:serialize',
    'digest_stop:worker:output',
    'digest_stop:worker:open:error',
    'digest_stop:worker:open:busy',
    'digest_stop:worker:ingest:error',
    'digest_stop:worker:ingest:busy',
    'digest_stop:worker:fitting:error',
    'digest_stop:worker:fitting:busy',
    'digest_stop:worker:claim:error',
    'digest_stop:worker:claim:busy',
    'digest_stop:worker:render:error',
    'digest_stop:worker:render:busy',
    'digest_stop:worker:close:error',
    'digest_stop:worker:close:busy',
    'digest_stop:launch:caller:config',
    'digest_stop:launch:caller:host',
    'digest_stop:launch:caller:pane',
    'digest_stop:launch:caller:storage',
    'digest_stop:launch:caller:storage_busy',
    'digest_stop:launch:caller:process',
    'digest_stop:launch:continuation:owner_probe',
    'digest_stop:launch:continuation:open:error',
    'digest_stop:launch:continuation:open:busy',
    'digest_stop:launch:continuation:write:error',
    'digest_stop:launch:continuation:write:busy',
    'digest_stop:launch:continuation:close:error',
    'digest_stop:launch:continuation:close:busy',
    'digest_stop:launch:continuation:read:error',
    'digest_stop:launch:continuation:read:busy'
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
