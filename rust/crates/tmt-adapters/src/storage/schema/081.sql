-- Current pending notifications only: Digest remains the held-item and attempt owner.
CREATE TABLE orchestration_pending_wakes (
 identity_id TEXT PRIMARY KEY NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
 released_classes INTEGER NOT NULL CHECK (typeof(released_classes)='integer' AND released_classes BETWEEN 1 AND 3),
 released_at_ms INTEGER NOT NULL CHECK (typeof(released_at_ms)='integer' AND released_at_ms BETWEEN 1 AND 9007199254740991),
 checklist_id TEXT REFERENCES focus_checklists(id) ON DELETE CASCADE
);
CREATE TABLE orchestration_wake_clock (
 id INTEGER PRIMARY KEY CHECK (id=1),
 next_admission_ms INTEGER NOT NULL CHECK (typeof(next_admission_ms)='integer' AND next_admission_ms BETWEEN 0 AND 9007199254740991),
 scan_after_identity_id TEXT REFERENCES identities(id) ON DELETE SET NULL
);
-- The singleton remains absent until actual wake admission or scan progress.
-- Historical checklists keep their existing full-content projection.
ALTER TABLE focus_checklists ADD COLUMN projection TEXT NOT NULL DEFAULT 'full'
 CHECK (projection IN ('full','incoming_summary'));
ALTER TABLE focus_checklists ADD COLUMN summary_urgent_count INTEGER CHECK (
 (projection='full' AND summary_urgent_count IS NULL) OR
 (projection='incoming_summary' AND typeof(sealed_count)='integer' AND sealed_count>=1
  AND typeof(summary_urgent_count)='integer' AND summary_urgent_count BETWEEN 0 AND sealed_count)
);
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
  OR OLD.projection IS NOT NEW.projection
  OR OLD.summary_urgent_count IS NOT NEW.summary_urgent_count
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER orchestration_pending_wakes_advances_change_cursor_on_insert AFTER INSERT ON orchestration_pending_wakes
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
CREATE TRIGGER orchestration_pending_wakes_advances_change_cursor_on_update AFTER UPDATE ON orchestration_pending_wakes
WHEN OLD.identity_id IS NOT NEW.identity_id OR OLD.released_classes IS NOT NEW.released_classes OR OLD.released_at_ms IS NOT NEW.released_at_ms OR OLD.checklist_id IS NOT NEW.checklist_id
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
CREATE TRIGGER orchestration_pending_wakes_advances_change_cursor_on_delete AFTER DELETE ON orchestration_pending_wakes
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
CREATE TRIGGER orchestration_wake_clock_advances_change_cursor_on_insert AFTER INSERT ON orchestration_wake_clock
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
CREATE TRIGGER orchestration_wake_clock_advances_change_cursor_on_update AFTER UPDATE ON orchestration_wake_clock
WHEN OLD.id IS NOT NEW.id OR OLD.next_admission_ms IS NOT NEW.next_admission_ms OR OLD.scan_after_identity_id IS NOT NEW.scan_after_identity_id
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
CREATE TRIGGER orchestration_wake_clock_advances_change_cursor_on_delete AFTER DELETE ON orchestration_wake_clock
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
