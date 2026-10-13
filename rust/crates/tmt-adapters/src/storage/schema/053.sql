-- Historical facts are unavailable: do not reconstruct old claim or settlement inputs.
ALTER TABLE focus_checklists ADD COLUMN opportunity TEXT
 CHECK (opportunity IS NULL OR opportunity IN ('idle', 'turn_boundary'));
ALTER TABLE focus_checklists ADD COLUMN sealed_count INTEGER
 CHECK (sealed_count IS NULL OR sealed_count BETWEEN 1 AND 9007199254740991);
ALTER TABLE focus_checklists ADD COLUMN settled_at_ms INTEGER
 CHECK (settled_at_ms IS NULL OR settled_at_ms BETWEEN 1 AND 9007199254740991);
ALTER TABLE focus_checklists ADD COLUMN settlement_sequence INTEGER
 CHECK (settlement_sequence IS NULL OR settlement_sequence BETWEEN 1 AND 9007199254740991);
CREATE UNIQUE INDEX focus_checklists_settlement_sequence ON focus_checklists(settlement_sequence)
 WHERE settlement_sequence IS NOT NULL;
CREATE INDEX focus_checklists_history ON focus_checklists(identity_id, settlement_sequence)
 WHERE settlement_sequence IS NOT NULL;
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
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
