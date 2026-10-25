-- Usage writes (the 5-second sampler's buckets, rollup, source and family rows) advance
-- their own cursor, not the change cursor (#5019). The change cursor is what wakes the
-- Ops board for roster, request, focus and cron changes; a sampler tick is none of those.
-- `tmt api changes.cursor` returns both. Triggers keep the compare-before-count rule of 040.
CREATE TABLE usage_cursor (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  value INTEGER NOT NULL CHECK (typeof(value) = 'integer' AND value >= 0)
);
INSERT INTO usage_cursor (id, value) VALUES (1, 0);
DROP TRIGGER consumption_sources_advances_change_cursor_on_insert;
DROP TRIGGER consumption_sources_advances_change_cursor_on_update;
DROP TRIGGER consumption_sources_advances_change_cursor_on_delete;
DROP TRIGGER consumption_buckets_advances_change_cursor_on_insert;
DROP TRIGGER consumption_buckets_advances_change_cursor_on_update;
DROP TRIGGER consumption_buckets_advances_change_cursor_on_delete;
DROP TRIGGER consumption_tier5m_advances_change_cursor_on_insert;
DROP TRIGGER consumption_tier5m_advances_change_cursor_on_update;
DROP TRIGGER consumption_tier5m_advances_change_cursor_on_delete;
DROP TRIGGER consumption_family_advances_change_cursor_on_insert;
DROP TRIGGER consumption_family_advances_change_cursor_on_update;
DROP TRIGGER consumption_family_advances_change_cursor_on_delete;
DROP TRIGGER consumption_family_files_advances_change_cursor_on_insert;
DROP TRIGGER consumption_family_files_advances_change_cursor_on_update;
DROP TRIGGER consumption_family_files_advances_change_cursor_on_delete;
CREATE TRIGGER consumption_sources_advances_usage_cursor_on_insert AFTER INSERT ON consumption_sources
BEGIN UPDATE usage_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER consumption_sources_advances_usage_cursor_on_update AFTER UPDATE ON consumption_sources
WHEN OLD.identity_id IS NOT NEW.identity_id OR OLD.binding_id IS NOT NEW.binding_id OR OLD.driver IS NOT NEW.driver OR OLD.session IS NOT NEW.session OR OLD.locator IS NOT NEW.locator OR OLD.sampled_at_ms IS NOT NEW.sampled_at_ms OR OLD.latest IS NOT NEW.latest
BEGIN UPDATE usage_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER consumption_sources_advances_usage_cursor_on_delete AFTER DELETE ON consumption_sources
BEGIN UPDATE usage_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER consumption_buckets_advances_usage_cursor_on_insert AFTER INSERT ON consumption_buckets
BEGIN UPDATE usage_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER consumption_buckets_advances_usage_cursor_on_update AFTER UPDATE ON consumption_buckets
WHEN OLD.identity_id IS NOT NEW.identity_id OR OLD.from_ms IS NOT NEW.from_ms OR OLD.input_tokens IS NOT NEW.input_tokens OR OLD.output_tokens IS NOT NEW.output_tokens OR OLD.cached_input_tokens IS NOT NEW.cached_input_tokens OR OLD.covered_ms IS NOT NEW.covered_ms OR OLD.complete IS NOT NEW.complete OR OLD.gap IS NOT NEW.gap OR OLD.discontinuous IS NOT NEW.discontinuous OR OLD.sampled_at_ms IS NOT NEW.sampled_at_ms OR OLD.latest IS NOT NEW.latest OR OLD.details IS NOT NEW.details
BEGIN UPDATE usage_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER consumption_buckets_advances_usage_cursor_on_delete AFTER DELETE ON consumption_buckets
BEGIN UPDATE usage_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER consumption_tier5m_advances_usage_cursor_on_insert AFTER INSERT ON consumption_tier5m
BEGIN UPDATE usage_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER consumption_tier5m_advances_usage_cursor_on_update AFTER UPDATE ON consumption_tier5m
WHEN OLD.identity_id IS NOT NEW.identity_id OR OLD.from_ms IS NOT NEW.from_ms OR OLD.input_tokens IS NOT NEW.input_tokens OR OLD.output_tokens IS NOT NEW.output_tokens OR OLD.cached_input_tokens IS NOT NEW.cached_input_tokens OR OLD.covered_ms IS NOT NEW.covered_ms OR OLD.complete IS NOT NEW.complete OR OLD.gap IS NOT NEW.gap OR OLD.discontinuous IS NOT NEW.discontinuous OR OLD.sampled_at_ms IS NOT NEW.sampled_at_ms OR OLD.limits IS NOT NEW.limits OR OLD.details IS NOT NEW.details OR OLD.credits IS NOT NEW.credits OR OLD.service_tier IS NOT NEW.service_tier
BEGIN UPDATE usage_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER consumption_tier5m_advances_usage_cursor_on_delete AFTER DELETE ON consumption_tier5m
BEGIN UPDATE usage_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER consumption_family_advances_usage_cursor_on_insert AFTER INSERT ON consumption_family
BEGIN UPDATE usage_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER consumption_family_advances_usage_cursor_on_update AFTER UPDATE ON consumption_family
WHEN OLD.identity_id IS NOT NEW.identity_id OR OLD.session IS NOT NEW.session OR OLD.scan IS NOT NEW.scan OR OLD.edge IS NOT NEW.edge OR OLD.closed IS NOT NEW.closed OR OLD.warm IS NOT NEW.warm
BEGIN UPDATE usage_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER consumption_family_advances_usage_cursor_on_delete AFTER DELETE ON consumption_family
BEGIN UPDATE usage_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER consumption_family_files_advances_usage_cursor_on_insert AFTER INSERT ON consumption_family_files
BEGIN UPDATE usage_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER consumption_family_files_advances_usage_cursor_on_update AFTER UPDATE ON consumption_family_files
WHEN OLD.identity_id IS NOT NEW.identity_id OR OLD.file IS NOT NEW.file OR OLD.child IS NOT NEW.child OR OLD.parent IS NOT NEW.parent OR OLD.base IS NOT NEW.base OR OLD.state IS NOT NEW.state OR OLD.touched_ms IS NOT NEW.touched_ms
BEGIN UPDATE usage_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER consumption_family_files_advances_usage_cursor_on_delete AFTER DELETE ON consumption_family_files
BEGIN UPDATE usage_cursor SET value = value + 1 WHERE id = 1; END;
