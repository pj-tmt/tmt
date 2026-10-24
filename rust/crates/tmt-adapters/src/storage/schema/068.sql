-- A checklist typed at an idle opportunity is unconfirmed until a prompt carries it (#4858):
-- NULL means still unconfirmed. Every checklist settled before this migration is confirmed at its
-- settlement, as is every turn-boundary delivery, which the provider's own hook output carries.
ALTER TABLE focus_checklists ADD COLUMN confirmed_at_ms INTEGER
 CHECK (confirmed_at_ms IS NULL OR confirmed_at_ms BETWEEN 1 AND 9007199254740991);
UPDATE focus_checklists SET confirmed_at_ms = COALESCE(settled_at_ms, created_at_ms)
 WHERE state = 'delivered' AND COALESCE(settled_at_ms, created_at_ms) >= 1;
-- A confirmation is a change like any other: readers waiting on the cursor must wake.
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
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
