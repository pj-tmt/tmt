-- Inactive driver sessions never describe a live binding. The current record remains
-- in identity_session_preferences, so existing installations require no backfill.
CREATE TABLE identity_resume_records (
  identity_id TEXT NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
  harness TEXT NOT NULL,
  runtime_mode TEXT NOT NULL,
  provider_session_id TEXT NOT NULL,
  driver_state_version INTEGER,
  driver_state TEXT,
  stale_at_ms INTEGER CHECK (stale_at_ms IS NULL OR stale_at_ms BETWEEN 0 AND 9007199254740991),
  resume_pending_at_ms INTEGER CHECK (resume_pending_at_ms IS NULL OR resume_pending_at_ms BETWEEN 0 AND 9007199254740991),
  channel INTEGER CHECK (channel IS NULL OR channel IN (0,1)),
  PRIMARY KEY (identity_id,harness),
  CHECK ((driver_state IS NULL) = (driver_state_version IS NULL)),
  CHECK (driver_state IS NULL OR (driver_state_version BETWEEN 1 AND 65535
    AND length(CAST(driver_state AS BLOB)) BETWEEN 1 AND 1024))
);
CREATE TRIGGER identity_resume_records_advances_change_cursor_on_insert
AFTER INSERT ON identity_resume_records
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_resume_records_advances_change_cursor_on_update
AFTER UPDATE ON identity_resume_records
WHEN OLD.identity_id IS NOT NEW.identity_id OR OLD.harness IS NOT NEW.harness
 OR OLD.runtime_mode IS NOT NEW.runtime_mode OR OLD.provider_session_id IS NOT NEW.provider_session_id
 OR OLD.driver_state_version IS NOT NEW.driver_state_version OR OLD.driver_state IS NOT NEW.driver_state
 OR OLD.stale_at_ms IS NOT NEW.stale_at_ms OR OLD.resume_pending_at_ms IS NOT NEW.resume_pending_at_ms
 OR OLD.channel IS NOT NEW.channel
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER identity_resume_records_advances_change_cursor_on_delete
AFTER DELETE ON identity_resume_records
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
