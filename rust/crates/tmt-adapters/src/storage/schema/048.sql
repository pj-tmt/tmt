ALTER TABLE request_attempts ADD COLUMN withdrawn_at_ms INTEGER
    CHECK (withdrawn_at_ms IS NULL OR withdrawn_at_ms BETWEEN 1 AND 9007199254740991)
    CHECK (withdrawn_at_ms IS NULL OR response_submitted_at_ms IS NULL);
ALTER TABLE request_attempts ADD COLUMN withdrawal_reason TEXT
    CHECK ((withdrawn_at_ms IS NULL AND withdrawal_reason IS NULL)
        OR (withdrawn_at_ms IS NOT NULL AND withdrawal_reason IS NOT NULL
            AND length(CAST(withdrawal_reason AS BLOB)) BETWEEN 1 AND 1024));

DROP TRIGGER request_attempts_advances_change_cursor_on_update;
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
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
