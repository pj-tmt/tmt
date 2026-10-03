-- Driver-owned source locators and accepted normalized observations, not content.
CREATE TABLE consumption_sources (
 identity_id TEXT PRIMARY KEY REFERENCES identities(id),
 binding_id TEXT NOT NULL REFERENCES bindings(id) ON DELETE CASCADE,
 driver TEXT NOT NULL CHECK (length(driver) BETWEEN 1 AND 64),
 session TEXT NOT NULL CHECK (length(session) BETWEEN 1 AND 256),
 locator TEXT CHECK (locator IS NULL OR length(CAST(locator AS BLOB)) BETWEEN 1 AND 4096),
 sampled_at_ms INTEGER CHECK (sampled_at_ms IS NULL OR (typeof(sampled_at_ms) = 'integer' AND sampled_at_ms > 0)),
 latest TEXT CHECK (latest IS NULL OR length(CAST(latest AS BLOB)) <= 2048)
);
CREATE TABLE consumption_buckets (
 identity_id TEXT NOT NULL REFERENCES identities(id),
 from_ms INTEGER NOT NULL CHECK (typeof(from_ms) = 'integer' AND from_ms >= 0 AND from_ms % 5000 = 0),
 input_tokens INTEGER NOT NULL CHECK (typeof(input_tokens) = 'integer' AND input_tokens >= 0),
 output_tokens INTEGER NOT NULL CHECK (typeof(output_tokens) = 'integer' AND output_tokens >= 0 AND input_tokens + output_tokens <= 9007199254740991),
 cached_input_tokens INTEGER NOT NULL CHECK (typeof(cached_input_tokens) = 'integer' AND cached_input_tokens BETWEEN 0 AND input_tokens),
 covered_ms INTEGER NOT NULL CHECK (typeof(covered_ms) = 'integer' AND covered_ms BETWEEN 0 AND 5000),
 complete INTEGER NOT NULL CHECK (complete IN (0,1)),
 gap INTEGER NOT NULL CHECK (gap IN (0,1)),
 discontinuous INTEGER NOT NULL CHECK (discontinuous IN (0,1)),
 sampled_at_ms INTEGER,
 latest TEXT CHECK (latest IS NULL OR length(CAST(latest AS BLOB)) <= 2048),
 PRIMARY KEY(identity_id, from_ms)
);
CREATE INDEX consumption_buckets_expiry ON consumption_buckets(from_ms);
CREATE TRIGGER consumption_sources_advances_change_cursor_on_insert AFTER INSERT ON consumption_sources
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER consumption_sources_advances_change_cursor_on_update AFTER UPDATE ON consumption_sources
WHEN OLD.identity_id IS NOT NEW.identity_id OR OLD.binding_id IS NOT NEW.binding_id OR OLD.driver IS NOT NEW.driver OR OLD.session IS NOT NEW.session OR OLD.locator IS NOT NEW.locator OR OLD.sampled_at_ms IS NOT NEW.sampled_at_ms OR OLD.latest IS NOT NEW.latest
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER consumption_sources_advances_change_cursor_on_delete AFTER DELETE ON consumption_sources
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER consumption_buckets_advances_change_cursor_on_insert AFTER INSERT ON consumption_buckets
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER consumption_buckets_advances_change_cursor_on_update AFTER UPDATE ON consumption_buckets
WHEN OLD.identity_id IS NOT NEW.identity_id OR OLD.from_ms IS NOT NEW.from_ms OR OLD.input_tokens IS NOT NEW.input_tokens OR OLD.output_tokens IS NOT NEW.output_tokens OR OLD.cached_input_tokens IS NOT NEW.cached_input_tokens OR OLD.covered_ms IS NOT NEW.covered_ms OR OLD.complete IS NOT NEW.complete OR OLD.gap IS NOT NEW.gap OR OLD.discontinuous IS NOT NEW.discontinuous OR OLD.sampled_at_ms IS NOT NEW.sampled_at_ms OR OLD.latest IS NOT NEW.latest
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER consumption_buckets_advances_change_cursor_on_delete AFTER DELETE ON consumption_buckets
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
