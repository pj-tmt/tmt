-- How long an agent's own answers stay readable (#4914). The chat shows every answer for the time
-- its person is away, so an answer is kept 7 days (capped at 1,000 per agent, oldest first) up to
-- 16 KiB, with the length the provider gave. The table is rebuilt because its text bound is a CHECK.
CREATE TABLE recent_responses_v71 (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
 identity_id TEXT NOT NULL REFERENCES identities(id),
 text TEXT NOT NULL CHECK (length(CAST(text AS BLOB)) BETWEEN 1 AND 16384),
 ended_ms INTEGER NOT NULL CHECK (typeof(ended_ms) = 'integer' AND ended_ms BETWEEN 1 AND 9007199254740991),
 provider TEXT NOT NULL CHECK (provider IN ('claude','codex')),
 truncated INTEGER NOT NULL CHECK (truncated IN (0,1)),
 session_id TEXT,
 expires_ms INTEGER NOT NULL CHECK (typeof(expires_ms) = 'integer' AND expires_ms > ended_ms AND expires_ms <= 9007199254740991),
 original_bytes INTEGER NOT NULL CHECK (typeof(original_bytes) = 'integer' AND original_bytes >= length(CAST(text AS BLOB)) AND (truncated = 1 OR original_bytes = length(CAST(text AS BLOB))))
);
-- A kept answer is held for the new 7 days at most; one captured before this migration cannot
-- know its original length, so it reports the stored length.
INSERT INTO recent_responses_v71(sequence,identity_id,text,ended_ms,provider,truncated,session_id,expires_ms,original_bytes)
 SELECT sequence,identity_id,text,ended_ms,provider,truncated,session_id,
        MIN(expires_ms, ended_ms + 604800000), length(CAST(text AS BLOB))
 FROM recent_responses;
DROP TABLE recent_responses;
ALTER TABLE recent_responses_v71 RENAME TO recent_responses;
CREATE INDEX recent_responses_identity ON recent_responses(identity_id,sequence DESC);
CREATE TRIGGER recent_responses_advances_change_cursor_on_insert AFTER INSERT ON recent_responses
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER recent_responses_advances_change_cursor_on_update AFTER UPDATE ON recent_responses
WHEN OLD.sequence IS NOT NEW.sequence OR OLD.identity_id IS NOT NEW.identity_id OR OLD.text IS NOT NEW.text OR OLD.ended_ms IS NOT NEW.ended_ms OR OLD.provider IS NOT NEW.provider OR OLD.truncated IS NOT NEW.truncated OR OLD.session_id IS NOT NEW.session_id OR OLD.expires_ms IS NOT NEW.expires_ms OR OLD.original_bytes IS NOT NEW.original_bytes
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER recent_responses_advances_change_cursor_on_delete AFTER DELETE ON recent_responses
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
-- The newest answer a writer pruned, so a read can say that older answers were not kept instead
-- of guessing. An identity that was at the old cap of 10 had older answers pruned already.
CREATE TABLE recent_response_floor (
 identity_id TEXT PRIMARY KEY REFERENCES identities(id),
 dropped_through_ms INTEGER NOT NULL CHECK (typeof(dropped_through_ms) = 'integer' AND dropped_through_ms BETWEEN 1 AND 9007199254740991)
);
INSERT INTO recent_response_floor(identity_id,dropped_through_ms)
 SELECT identity_id, MIN(ended_ms) FROM recent_responses GROUP BY identity_id HAVING count(*) >= 10;
CREATE TRIGGER recent_response_floor_advances_change_cursor_on_insert AFTER INSERT ON recent_response_floor
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER recent_response_floor_advances_change_cursor_on_update AFTER UPDATE ON recent_response_floor
WHEN OLD.identity_id IS NOT NEW.identity_id OR OLD.dropped_through_ms IS NOT NEW.dropped_through_ms
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER recent_response_floor_advances_change_cursor_on_delete AFTER DELETE ON recent_response_floor
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
