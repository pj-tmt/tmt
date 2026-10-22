-- How long the Digest rule's owner keeps a member's oldest held message at most. The rule
-- itself stays with the Digest extension; recording its limit with the hold lets Core state
-- the latest moment a held message goes out. Older policies read null until next written.
ALTER TABLE focus_policies
    ADD COLUMN max_hold_ms INTEGER CHECK (max_hold_ms IS NULL OR max_hold_ms BETWEEN 1 AND 9007199254740991);

DROP TRIGGER focus_policies_advances_change_cursor_on_update;
CREATE TRIGGER focus_policies_advances_change_cursor_on_update AFTER UPDATE ON focus_policies
WHEN OLD.identity_id IS NOT NEW.identity_id
  OR OLD.revision IS NOT NEW.revision
  OR OLD.until_ms IS NOT NEW.until_ms
  OR OLD.owner_identity_id IS NOT NEW.owner_identity_id
  OR OLD.setter_identity_id IS NOT NEW.setter_identity_id
  OR OLD.max_hold_ms IS NOT NEW.max_hold_ms
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
