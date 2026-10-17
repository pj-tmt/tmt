-- Local provider-final pane output, separate from exchanges and opaque driver state.
-- Sequence, not wall time, defines capture order even across clock rollback.
CREATE TABLE recent_responses (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
 identity_id TEXT NOT NULL REFERENCES identities(id),
 text TEXT NOT NULL CHECK (length(CAST(text AS BLOB)) BETWEEN 1 AND 8192),
 ended_ms INTEGER NOT NULL CHECK (typeof(ended_ms) = 'integer' AND ended_ms BETWEEN 1 AND 9007199254740991),
 provider TEXT NOT NULL CHECK (provider IN ('claude','codex')),
 truncated INTEGER NOT NULL CHECK (truncated IN (0,1)),
 session_id TEXT,
 expires_ms INTEGER NOT NULL CHECK (typeof(expires_ms) = 'integer' AND expires_ms > ended_ms AND expires_ms <= 9007199254740991)
);
CREATE INDEX recent_responses_identity ON recent_responses(identity_id,sequence DESC);
CREATE TRIGGER recent_responses_advances_change_cursor_on_insert AFTER INSERT ON recent_responses
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER recent_responses_advances_change_cursor_on_update AFTER UPDATE ON recent_responses
WHEN OLD.sequence IS NOT NEW.sequence OR OLD.identity_id IS NOT NEW.identity_id OR OLD.text IS NOT NEW.text OR OLD.ended_ms IS NOT NEW.ended_ms OR OLD.provider IS NOT NEW.provider OR OLD.truncated IS NOT NEW.truncated OR OLD.session_id IS NOT NEW.session_id OR OLD.expires_ms IS NOT NEW.expires_ms
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER recent_responses_advances_change_cursor_on_delete AFTER DELETE ON recent_responses
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
