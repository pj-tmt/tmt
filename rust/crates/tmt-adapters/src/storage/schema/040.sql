-- A change cursor for API consumers (#509). One counter advances on every
-- committed insert, update or delete of core-owned durable records, so a
-- consumer can learn "something changed" with one read instead of reloading.
-- It is opaque: consumers compare it for equality only.
--
-- Covered: every core-owned table. Not covered: migration bookkeeping
-- (_migrations, extension_storage_cutovers), the counter itself, and tables
-- whose data belongs to an extension (the storage-cutover set). Refreshing a
-- binding's last_verified_at is an observation made by reads, not a change.
-- An update counts only when a compared column's value actually differs, so
-- a statement that rewrites a row with the same values is not a change. A
-- migration that adds a column to, or rebuilds, a covered table must recreate
-- its triggers; the schema tests fail until it does.
CREATE TABLE change_cursor (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  value INTEGER NOT NULL CHECK (typeof(value) = 'integer' AND value >= 0)
);
INSERT INTO change_cursor (id, value) VALUES (1, 0);
CREATE TRIGGER bindings_advances_change_cursor_on_insert AFTER INSERT ON bindings
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
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
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER bindings_advances_change_cursor_on_delete AFTER DELETE ON bindings
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER host_servers_advances_change_cursor_on_insert AFTER INSERT ON host_servers
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER host_servers_advances_change_cursor_on_update AFTER UPDATE ON host_servers
WHEN OLD.server_id IS NOT NEW.server_id
   OR OLD.host IS NOT NEW.host
   OR OLD.socket_path IS NOT NEW.socket_path
   OR OLD.server_pid IS NOT NEW.server_pid
   OR OLD.server_start_time IS NOT NEW.server_start_time
   OR OLD.first_seen_at_ms IS NOT NEW.first_seen_at_ms
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER host_servers_advances_change_cursor_on_delete AFTER DELETE ON host_servers
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identities_advances_change_cursor_on_insert AFTER INSERT ON identities
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identities_advances_change_cursor_on_update AFTER UPDATE ON identities
WHEN OLD.id IS NOT NEW.id
   OR OLD.name IS NOT NEW.name
   OR OLD.canonical_name IS NOT NEW.canonical_name
   OR OLD.created_at IS NOT NEW.created_at
   OR OLD.updated_at IS NOT NEW.updated_at
   OR OLD.lifetime IS NOT NEW.lifetime
   OR OLD.retired_at_ms IS NOT NEW.retired_at_ms
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identities_advances_change_cursor_on_delete AFTER DELETE ON identities
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_hooks_advances_change_cursor_on_insert AFTER INSERT ON identity_hooks
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_hooks_advances_change_cursor_on_update AFTER UPDATE ON identity_hooks
WHEN OLD.consumer IS NOT NEW.consumer
   OR OLD.identity_id IS NOT NEW.identity_id
   OR OLD.reference IS NOT NEW.reference
   OR OLD.state IS NOT NEW.state
   OR OLD.attempt_count IS NOT NEW.attempt_count
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_hooks_advances_change_cursor_on_delete AFTER DELETE ON identity_hooks
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_metadata_advances_change_cursor_on_insert AFTER INSERT ON identity_metadata
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_metadata_advances_change_cursor_on_update AFTER UPDATE ON identity_metadata
WHEN OLD.identity_id IS NOT NEW.identity_id
   OR OLD.key IS NOT NEW.key
   OR OLD.value IS NOT NEW.value
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_metadata_advances_change_cursor_on_delete AFTER DELETE ON identity_metadata
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_preambles_advances_change_cursor_on_insert AFTER INSERT ON identity_preambles
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_preambles_advances_change_cursor_on_update AFTER UPDATE ON identity_preambles
WHEN OLD.identity_id IS NOT NEW.identity_id
   OR OLD.content IS NOT NEW.content
   OR OLD.updated_at IS NOT NEW.updated_at
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_preambles_advances_change_cursor_on_delete AFTER DELETE ON identity_preambles
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_session_preferences_advances_change_cursor_on_insert AFTER INSERT ON identity_session_preferences
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_session_preferences_advances_change_cursor_on_update AFTER UPDATE ON identity_session_preferences
WHEN OLD.identity_id IS NOT NEW.identity_id
   OR OLD.preferred_harness IS NOT NEW.preferred_harness
   OR OLD.remembered_harness IS NOT NEW.remembered_harness
   OR OLD.runtime_mode IS NOT NEW.runtime_mode
   OR OLD.provider_session_id IS NOT NEW.provider_session_id
   OR OLD.driver_state IS NOT NEW.driver_state
   OR OLD.driver_state_version IS NOT NEW.driver_state_version
   OR OLD.stale_at_ms IS NOT NEW.stale_at_ms
   OR OLD.resume_pending_at_ms IS NOT NEW.resume_pending_at_ms
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_session_preferences_advances_change_cursor_on_delete AFTER DELETE ON identity_session_preferences
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_status_advances_change_cursor_on_insert AFTER INSERT ON identity_status
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_status_advances_change_cursor_on_update AFTER UPDATE ON identity_status
WHEN OLD.identity_id IS NOT NEW.identity_id
   OR OLD.activity IS NOT NEW.activity
   OR OLD.mood IS NOT NEW.mood
   OR OLD.updated_at_ms IS NOT NEW.updated_at_ms
   OR OLD.expires_at_ms IS NOT NEW.expires_at_ms
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_status_advances_change_cursor_on_delete AFTER DELETE ON identity_status
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER office_dispatch_operations_advances_change_cursor_on_insert AFTER INSERT ON office_dispatch_operations
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER office_dispatch_operations_advances_change_cursor_on_update AFTER UPDATE ON office_dispatch_operations
WHEN OLD.operation_id IS NOT NEW.operation_id
   OR OLD.intent_digest IS NOT NEW.intent_digest
   OR OLD.receipt IS NOT NEW.receipt
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER office_dispatch_operations_advances_change_cursor_on_delete AFTER DELETE ON office_dispatch_operations
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER office_meeting_members_advances_change_cursor_on_insert AFTER INSERT ON office_meeting_members
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER office_meeting_members_advances_change_cursor_on_update AFTER UPDATE ON office_meeting_members
WHEN OLD.room_id IS NOT NEW.room_id
   OR OLD.identity_id IS NOT NEW.identity_id
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER office_meeting_members_advances_change_cursor_on_delete AFTER DELETE ON office_meeting_members
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER office_meeting_rooms_advances_change_cursor_on_insert AFTER INSERT ON office_meeting_rooms
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER office_meeting_rooms_advances_change_cursor_on_update AFTER UPDATE ON office_meeting_rooms
WHEN OLD.room_id IS NOT NEW.room_id
   OR OLD.name IS NOT NEW.name
   OR OLD.revision IS NOT NEW.revision
   OR OLD.retired IS NOT NEW.retired
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER office_meeting_rooms_advances_change_cursor_on_delete AFTER DELETE ON office_meeting_rooms
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER preamble_counters_advances_change_cursor_on_insert AFTER INSERT ON preamble_counters
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER preamble_counters_advances_change_cursor_on_update AFTER UPDATE ON preamble_counters
WHEN OLD.identity_id IS NOT NEW.identity_id
   OR OLD.reserved_count IS NOT NEW.reserved_count
   OR OLD.updated_at_ms IS NOT NEW.updated_at_ms
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER preamble_counters_advances_change_cursor_on_delete AFTER DELETE ON preamble_counters
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_attempts_advances_change_cursor_on_insert AFTER INSERT ON request_attempts
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
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_attempts_advances_change_cursor_on_delete AFTER DELETE ON request_attempts
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_attention_identities_advances_change_cursor_on_insert AFTER INSERT ON request_attention_identities
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_attention_identities_advances_change_cursor_on_update AFTER UPDATE ON request_attention_identities
WHEN OLD.identity_id IS NOT NEW.identity_id
   OR OLD.latest_revision IS NOT NEW.latest_revision
   OR OLD.acknowledged_through IS NOT NEW.acknowledged_through
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_attention_identities_advances_change_cursor_on_delete AFTER DELETE ON request_attention_identities
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_notifications_advances_change_cursor_on_insert AFTER INSERT ON request_notifications
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_notifications_advances_change_cursor_on_update AFTER UPDATE ON request_notifications
WHEN OLD.request_id IS NOT NEW.request_id
   OR OLD.deadline_ms IS NOT NEW.deadline_ms
   OR OLD.timeout_ms IS NOT NEW.timeout_ms
   OR OLD.waiter_pid IS NOT NEW.waiter_pid
   OR OLD.waiter_start IS NOT NEW.waiter_start
   OR OLD.reply_state IS NOT NEW.reply_state
   OR OLD.timeout_state IS NOT NEW.timeout_state
   OR OLD.observed IS NOT NEW.observed
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_notifications_advances_change_cursor_on_delete AFTER DELETE ON request_notifications
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_recipient_attention_identities_advances_change_cursor_on_insert AFTER INSERT ON request_recipient_attention_identities
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_recipient_attention_identities_advances_change_cursor_on_update AFTER UPDATE ON request_recipient_attention_identities
WHEN OLD.identity_id IS NOT NEW.identity_id
   OR OLD.latest_revision IS NOT NEW.latest_revision
   OR OLD.acknowledged_through IS NOT NEW.acknowledged_through
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_recipient_attention_identities_advances_change_cursor_on_delete AFTER DELETE ON request_recipient_attention_identities
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_responses_advances_change_cursor_on_insert AFTER INSERT ON request_responses
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
CREATE TRIGGER request_responses_advances_change_cursor_on_delete AFTER DELETE ON request_responses
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER role_profiles_advances_change_cursor_on_insert AFTER INSERT ON role_profiles
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER role_profiles_advances_change_cursor_on_update AFTER UPDATE ON role_profiles
WHEN OLD.identity_id IS NOT NEW.identity_id
   OR OLD.content IS NOT NEW.content
   OR OLD.updated_at IS NOT NEW.updated_at
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER role_profiles_advances_change_cursor_on_delete AFTER DELETE ON role_profiles
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
