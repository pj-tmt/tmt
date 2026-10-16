-- Five-minute rollup of the 5-second consumption buckets, kept 30 days. Written in the
-- same transaction as each accepted read, so it is a sum of the fine buckets, not a second
-- ledger; recording starts here, with no backfill. `limits` is the newest rate-limit object
-- observed inside the bucket. Like every core table it advances the change cursor.
CREATE TABLE consumption_tier5m (
 identity_id TEXT NOT NULL REFERENCES identities(id),
 from_ms INTEGER NOT NULL CHECK (typeof(from_ms) = 'integer' AND from_ms >= 0 AND from_ms % 300000 = 0),
 input_tokens INTEGER NOT NULL CHECK (typeof(input_tokens) = 'integer' AND input_tokens >= 0),
 output_tokens INTEGER NOT NULL CHECK (typeof(output_tokens) = 'integer' AND output_tokens >= 0 AND input_tokens + output_tokens <= 9007199254740991),
 cached_input_tokens INTEGER NOT NULL CHECK (typeof(cached_input_tokens) = 'integer' AND cached_input_tokens BETWEEN 0 AND input_tokens),
 covered_ms INTEGER NOT NULL CHECK (typeof(covered_ms) = 'integer' AND covered_ms BETWEEN 0 AND 300000),
 complete INTEGER NOT NULL CHECK (complete IN (0,1)),
 gap INTEGER NOT NULL CHECK (gap IN (0,1)),
 discontinuous INTEGER NOT NULL CHECK (discontinuous IN (0,1)),
 sampled_at_ms INTEGER,
 limits TEXT CHECK (limits IS NULL OR length(CAST(limits AS BLOB)) <= 256),
 details TEXT CHECK (details IS NULL OR length(CAST(details AS BLOB)) <= 2048),
 PRIMARY KEY(identity_id, from_ms)
) WITHOUT ROWID;
CREATE INDEX consumption_tier5m_expiry ON consumption_tier5m(from_ms);
CREATE TRIGGER consumption_tier5m_advances_change_cursor_on_insert AFTER INSERT ON consumption_tier5m
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER consumption_tier5m_advances_change_cursor_on_update AFTER UPDATE ON consumption_tier5m
WHEN OLD.identity_id IS NOT NEW.identity_id OR OLD.from_ms IS NOT NEW.from_ms OR OLD.input_tokens IS NOT NEW.input_tokens OR OLD.output_tokens IS NOT NEW.output_tokens OR OLD.cached_input_tokens IS NOT NEW.cached_input_tokens OR OLD.covered_ms IS NOT NEW.covered_ms OR OLD.complete IS NOT NEW.complete OR OLD.gap IS NOT NEW.gap OR OLD.discontinuous IS NOT NEW.discontinuous OR OLD.sampled_at_ms IS NOT NEW.sampled_at_ms OR OLD.limits IS NOT NEW.limits OR OLD.details IS NOT NEW.details
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER consumption_tier5m_advances_change_cursor_on_delete AFTER DELETE ON consumption_tier5m
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
