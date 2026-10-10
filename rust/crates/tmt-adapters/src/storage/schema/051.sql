-- Existing references have no recorded arrival observation; preserve that absence.
ALTER TABLE focus_items ADD COLUMN context_tokens_at_arrival INTEGER
 CHECK (context_tokens_at_arrival IS NULL OR context_tokens_at_arrival BETWEEN 0 AND 9007199254740991);
ALTER TABLE focus_items ADD COLUMN context_observed_at_ms INTEGER
 CHECK ((context_tokens_at_arrival IS NULL AND context_observed_at_ms IS NULL)
     OR (context_tokens_at_arrival IS NOT NULL AND context_observed_at_ms IS NOT NULL AND context_observed_at_ms BETWEEN 1 AND 9007199254740991));
DROP TRIGGER focus_items_advances_change_cursor_on_update;
CREATE TRIGGER focus_items_advances_change_cursor_on_update AFTER UPDATE ON focus_items
WHEN OLD.sequence IS NOT NEW.sequence
  OR OLD.identity_id IS NOT NEW.identity_id
  OR OLD.request_id IS NOT NEW.request_id
  OR OLD.kind IS NOT NEW.kind
  OR OLD.source IS NOT NEW.source
  OR OLD.created_at_ms IS NOT NEW.created_at_ms
  OR OLD.checklist_id IS NOT NEW.checklist_id
  OR OLD.context_tokens_at_arrival IS NOT NEW.context_tokens_at_arrival
  OR OLD.context_observed_at_ms IS NOT NEW.context_observed_at_ms
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;

CREATE TABLE digest_counters (
    identity_id TEXT PRIMARY KEY REFERENCES identities(id) ON DELETE CASCADE,
    due_through_sequence INTEGER NOT NULL DEFAULT 0 CHECK (due_through_sequence BETWEEN 0 AND 9007199254740991),
    delivered_digests INTEGER NOT NULL DEFAULT 0 CHECK (delivered_digests BETWEEN 0 AND 9007199254740991)
);
-- Seed only successful retained history. Already-pruned history cannot be reconstructed.
INSERT INTO digest_counters(identity_id, delivered_digests)
 SELECT identity_id, COUNT(*) FROM focus_checklists WHERE state = 'delivered' GROUP BY identity_id;
CREATE TRIGGER digest_counters_advances_change_cursor_on_insert AFTER INSERT ON digest_counters
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER digest_counters_advances_change_cursor_on_update AFTER UPDATE ON digest_counters
WHEN OLD.identity_id IS NOT NEW.identity_id
  OR OLD.due_through_sequence IS NOT NEW.due_through_sequence
  OR OLD.delivered_digests IS NOT NEW.delivered_digests
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER digest_counters_advances_change_cursor_on_delete AFTER DELETE ON digest_counters
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
