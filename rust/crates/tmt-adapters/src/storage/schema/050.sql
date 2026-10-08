-- Unknown legacy attribution stays NULL; do not backfill provider evidence.
ALTER TABLE consumption_buckets ADD COLUMN details TEXT
 CHECK (details IS NULL OR length(CAST(details AS BLOB)) <= 2048);
DROP TRIGGER consumption_buckets_advances_change_cursor_on_update;
CREATE TRIGGER consumption_buckets_advances_change_cursor_on_update AFTER UPDATE ON consumption_buckets
WHEN OLD.identity_id IS NOT NEW.identity_id OR OLD.from_ms IS NOT NEW.from_ms OR OLD.input_tokens IS NOT NEW.input_tokens OR OLD.output_tokens IS NOT NEW.output_tokens OR OLD.cached_input_tokens IS NOT NEW.cached_input_tokens OR OLD.covered_ms IS NOT NEW.covered_ms OR OLD.complete IS NOT NEW.complete OR OLD.gap IS NOT NEW.gap OR OLD.discontinuous IS NOT NEW.discontinuous OR OLD.sampled_at_ms IS NOT NEW.sampled_at_ms OR OLD.latest IS NOT NEW.latest OR OLD.details IS NOT NEW.details
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
