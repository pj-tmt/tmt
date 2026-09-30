-- A change cursor for API consumers (#509). One counter advances on every
-- committed insert, update or delete of core-owned durable records, so a
-- consumer can learn "something changed" with one read instead of reloading.
-- It is opaque: consumers compare it for equality only.
--
-- Covered: every core-owned table. Not covered: migration bookkeeping
-- (_migrations, extension_storage_cutovers), the counter itself, and tables
-- whose data belongs to an extension (the storage-cutover set). Refreshing a
-- binding's last_verified_at is an observation made by reads, not a change,
-- so bindings' update trigger names every other column. A migration that
-- rebuilds a covered table must recreate its three triggers; the schema
-- tests fail until it does.
CREATE TABLE change_cursor (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  value INTEGER NOT NULL CHECK (typeof(value) = 'integer' AND value >= 0)
);
INSERT INTO change_cursor (id, value) VALUES (1, 0);
CREATE TRIGGER bindings_advances_change_cursor_on_insert AFTER INSERT ON bindings
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER bindings_advances_change_cursor_on_update AFTER UPDATE OF id, identity_id, transport, pane_id, server_id, socket_path, server_pid, server_start_time, pane_pid, bound_at, runtime_state, last_transition, runtime_pid, runtime_start_identity, observed_provider_session_id, launch_owner_pid, launch_owner_start_identity ON bindings
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER bindings_advances_change_cursor_on_delete AFTER DELETE ON bindings
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER host_servers_advances_change_cursor_on_insert AFTER INSERT ON host_servers
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER host_servers_advances_change_cursor_on_update AFTER UPDATE ON host_servers
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER host_servers_advances_change_cursor_on_delete AFTER DELETE ON host_servers
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identities_advances_change_cursor_on_insert AFTER INSERT ON identities
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identities_advances_change_cursor_on_update AFTER UPDATE ON identities
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identities_advances_change_cursor_on_delete AFTER DELETE ON identities
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_hooks_advances_change_cursor_on_insert AFTER INSERT ON identity_hooks
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_hooks_advances_change_cursor_on_update AFTER UPDATE ON identity_hooks
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_hooks_advances_change_cursor_on_delete AFTER DELETE ON identity_hooks
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_metadata_advances_change_cursor_on_insert AFTER INSERT ON identity_metadata
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_metadata_advances_change_cursor_on_update AFTER UPDATE ON identity_metadata
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_metadata_advances_change_cursor_on_delete AFTER DELETE ON identity_metadata
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_preambles_advances_change_cursor_on_insert AFTER INSERT ON identity_preambles
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_preambles_advances_change_cursor_on_update AFTER UPDATE ON identity_preambles
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_preambles_advances_change_cursor_on_delete AFTER DELETE ON identity_preambles
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_session_preferences_advances_change_cursor_on_insert AFTER INSERT ON identity_session_preferences
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_session_preferences_advances_change_cursor_on_update AFTER UPDATE ON identity_session_preferences
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_session_preferences_advances_change_cursor_on_delete AFTER DELETE ON identity_session_preferences
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_status_advances_change_cursor_on_insert AFTER INSERT ON identity_status
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_status_advances_change_cursor_on_update AFTER UPDATE ON identity_status
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_status_advances_change_cursor_on_delete AFTER DELETE ON identity_status
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER office_dispatch_operations_advances_change_cursor_on_insert AFTER INSERT ON office_dispatch_operations
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER office_dispatch_operations_advances_change_cursor_on_update AFTER UPDATE ON office_dispatch_operations
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER office_dispatch_operations_advances_change_cursor_on_delete AFTER DELETE ON office_dispatch_operations
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER office_meeting_members_advances_change_cursor_on_insert AFTER INSERT ON office_meeting_members
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER office_meeting_members_advances_change_cursor_on_update AFTER UPDATE ON office_meeting_members
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER office_meeting_members_advances_change_cursor_on_delete AFTER DELETE ON office_meeting_members
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER office_meeting_rooms_advances_change_cursor_on_insert AFTER INSERT ON office_meeting_rooms
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER office_meeting_rooms_advances_change_cursor_on_update AFTER UPDATE ON office_meeting_rooms
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER office_meeting_rooms_advances_change_cursor_on_delete AFTER DELETE ON office_meeting_rooms
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER preamble_counters_advances_change_cursor_on_insert AFTER INSERT ON preamble_counters
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER preamble_counters_advances_change_cursor_on_update AFTER UPDATE ON preamble_counters
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER preamble_counters_advances_change_cursor_on_delete AFTER DELETE ON preamble_counters
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_attempts_advances_change_cursor_on_insert AFTER INSERT ON request_attempts
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_attempts_advances_change_cursor_on_update AFTER UPDATE ON request_attempts
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_attempts_advances_change_cursor_on_delete AFTER DELETE ON request_attempts
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_attention_identities_advances_change_cursor_on_insert AFTER INSERT ON request_attention_identities
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_attention_identities_advances_change_cursor_on_update AFTER UPDATE ON request_attention_identities
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_attention_identities_advances_change_cursor_on_delete AFTER DELETE ON request_attention_identities
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_notifications_advances_change_cursor_on_insert AFTER INSERT ON request_notifications
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_notifications_advances_change_cursor_on_update AFTER UPDATE ON request_notifications
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_notifications_advances_change_cursor_on_delete AFTER DELETE ON request_notifications
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_recipient_attention_identities_advances_change_cursor_on_insert AFTER INSERT ON request_recipient_attention_identities
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_recipient_attention_identities_advances_change_cursor_on_update AFTER UPDATE ON request_recipient_attention_identities
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_recipient_attention_identities_advances_change_cursor_on_delete AFTER DELETE ON request_recipient_attention_identities
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_responses_advances_change_cursor_on_insert AFTER INSERT ON request_responses
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_responses_advances_change_cursor_on_update AFTER UPDATE ON request_responses
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_responses_advances_change_cursor_on_delete AFTER DELETE ON request_responses
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER role_profiles_advances_change_cursor_on_insert AFTER INSERT ON role_profiles
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER role_profiles_advances_change_cursor_on_update AFTER UPDATE ON role_profiles
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER role_profiles_advances_change_cursor_on_delete AFTER DELETE ON role_profiles
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
