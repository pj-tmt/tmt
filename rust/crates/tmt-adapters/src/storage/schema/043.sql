-- Naming an automatic identity consumes provenance without replacing its binding.
ALTER TABLE identities ADD COLUMN auto_named INTEGER NOT NULL DEFAULT 0 CHECK (auto_named IN (0, 1));
DROP TRIGGER identities_advances_change_cursor_on_update;
CREATE TRIGGER identities_advances_change_cursor_on_update AFTER UPDATE ON identities
WHEN OLD.id IS NOT NEW.id
   OR OLD.name IS NOT NEW.name
   OR OLD.canonical_name IS NOT NEW.canonical_name
   OR OLD.created_at IS NOT NEW.created_at
   OR OLD.updated_at IS NOT NEW.updated_at
   OR OLD.lifetime IS NOT NEW.lifetime
   OR OLD.retired_at_ms IS NOT NEW.retired_at_ms
   OR OLD.auto_named IS NOT NEW.auto_named
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
