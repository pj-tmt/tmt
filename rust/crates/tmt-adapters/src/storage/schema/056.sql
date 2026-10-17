-- The newest credit balance and the service tier of the newest token-moving read inside
-- each five-minute rollup bucket (#3546). Additive: no existing row is rewritten, and
-- buckets from before this migration keep both absent. The update trigger is recreated
-- so a change to either advances the change cursor, as every other column does.
ALTER TABLE consumption_tier5m ADD COLUMN credits TEXT CHECK (credits IS NULL OR length(CAST(credits AS BLOB)) <= 256);
ALTER TABLE consumption_tier5m ADD COLUMN service_tier TEXT CHECK (service_tier IS NULL OR length(CAST(service_tier AS BLOB)) <= 24);
DROP TRIGGER consumption_tier5m_advances_change_cursor_on_update;
CREATE TRIGGER consumption_tier5m_advances_change_cursor_on_update AFTER UPDATE ON consumption_tier5m
WHEN OLD.identity_id IS NOT NEW.identity_id OR OLD.from_ms IS NOT NEW.from_ms OR OLD.input_tokens IS NOT NEW.input_tokens OR OLD.output_tokens IS NOT NEW.output_tokens OR OLD.cached_input_tokens IS NOT NEW.cached_input_tokens OR OLD.covered_ms IS NOT NEW.covered_ms OR OLD.complete IS NOT NEW.complete OR OLD.gap IS NOT NEW.gap OR OLD.discontinuous IS NOT NEW.discontinuous OR OLD.sampled_at_ms IS NOT NEW.sampled_at_ms OR OLD.limits IS NOT NEW.limits OR OLD.details IS NOT NEW.details OR OLD.credits IS NOT NEW.credits OR OLD.service_tier IS NOT NEW.service_tier
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
