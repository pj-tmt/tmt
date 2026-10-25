-- Digest publishes its configured receiving input; Core never reads its TOML.
-- A missing row means the input is unavailable, rather than implicitly direct.
CREATE TABLE digest_receiving_settings (
    id INTEGER PRIMARY KEY CHECK (id=1),
    revision INTEGER NOT NULL CHECK (typeof(revision)='integer' AND revision BETWEEN 1 AND 9007199254740991),
    settings TEXT NOT NULL CHECK (json_valid(settings) AND length(CAST(settings AS BLOB)) <= 131072)
);
CREATE TRIGGER digest_receiving_settings_advances_change_cursor_on_insert
AFTER INSERT ON digest_receiving_settings
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;

-- Claim provenance belongs to the retained checklist, independent of the
-- shorter journal retention. Settlement never reinterprets edited plans.
ALTER TABLE focus_checklists ADD COLUMN orchestration_context TEXT
 CHECK (orchestration_context IS NULL OR (json_valid(orchestration_context) AND length(CAST(orchestration_context AS BLOB)) <= 8192));
DROP TRIGGER focus_checklists_advances_change_cursor_on_update;
CREATE TRIGGER focus_checklists_advances_change_cursor_on_update AFTER UPDATE ON focus_checklists
WHEN OLD.id IS NOT NEW.id
  OR OLD.identity_id IS NOT NEW.identity_id
  OR OLD.attempt_token IS NOT NEW.attempt_token
  OR OLD.through_sequence IS NOT NEW.through_sequence
  OR OLD.state IS NOT NEW.state
  OR OLD.created_at_ms IS NOT NEW.created_at_ms
  OR OLD.opportunity IS NOT NEW.opportunity
  OR OLD.sealed_count IS NOT NEW.sealed_count
  OR OLD.settled_at_ms IS NOT NEW.settled_at_ms
  OR OLD.settlement_sequence IS NOT NEW.settlement_sequence
  OR OLD.confirmed_at_ms IS NOT NEW.confirmed_at_ms
  OR OLD.orchestration_context IS NOT NEW.orchestration_context
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER digest_receiving_settings_advances_change_cursor_on_update
AFTER UPDATE ON digest_receiving_settings
WHEN OLD.id IS NOT NEW.id OR OLD.revision IS NOT NEW.revision OR OLD.settings IS NOT NEW.settings
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
CREATE TRIGGER digest_receiving_settings_advances_change_cursor_on_delete
AFTER DELETE ON digest_receiving_settings
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
