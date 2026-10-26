-- Capture every full runtime's payload finals through the existing transaction. This rebuild
-- widens only the provider checks; retained IDs, answer links and expiry remain unchanged.
CREATE TEMP TABLE response_sequence_v79 AS SELECT seq FROM sqlite_sequence WHERE name = 'recent_responses';
CREATE TABLE recent_responses_v79 (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
 identity_id TEXT NOT NULL REFERENCES identities(id),
 text TEXT NOT NULL CHECK (length(CAST(text AS BLOB)) BETWEEN 1 AND 16384),
 ended_ms INTEGER NOT NULL CHECK (typeof(ended_ms) = 'integer' AND ended_ms BETWEEN 1 AND 9007199254740991),
 provider TEXT NOT NULL CHECK (provider IN ('agy','claude','codex')),
 truncated INTEGER NOT NULL CHECK (truncated IN (0,1)),
 session_id TEXT,
 expires_ms INTEGER NOT NULL CHECK (typeof(expires_ms) = 'integer' AND expires_ms > ended_ms AND expires_ms <= 9007199254740991),
 original_bytes INTEGER NOT NULL CHECK (typeof(original_bytes) = 'integer' AND original_bytes >= length(CAST(text AS BLOB)) AND (truncated = 1 OR original_bytes = length(CAST(text AS BLOB))))
);
INSERT INTO recent_responses_v79(sequence,identity_id,text,ended_ms,provider,truncated,session_id,expires_ms,original_bytes)
 SELECT sequence,identity_id,text,ended_ms,provider,truncated,session_id,expires_ms,original_bytes
 FROM recent_responses;
DROP TABLE recent_responses;
ALTER TABLE recent_responses_v79 RENAME TO recent_responses;
-- Copying surviving rows alone loses the high-water mark when a newer final was pruned.
UPDATE sqlite_sequence SET seq = MAX(seq, COALESCE((SELECT seq FROM response_sequence_v79), seq))
 WHERE name = 'recent_responses';
INSERT INTO sqlite_sequence(name,seq)
 SELECT 'recent_responses',seq FROM response_sequence_v79
 WHERE seq IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name = 'recent_responses');
DROP TABLE response_sequence_v79;
CREATE INDEX recent_responses_identity ON recent_responses(identity_id,sequence DESC);
CREATE TRIGGER recent_responses_advances_change_cursor_on_insert AFTER INSERT ON recent_responses
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER recent_responses_advances_change_cursor_on_update AFTER UPDATE ON recent_responses
WHEN OLD.sequence IS NOT NEW.sequence OR OLD.identity_id IS NOT NEW.identity_id OR OLD.text IS NOT NEW.text OR OLD.ended_ms IS NOT NEW.ended_ms OR OLD.provider IS NOT NEW.provider OR OLD.truncated IS NOT NEW.truncated OR OLD.session_id IS NOT NEW.session_id OR OLD.expires_ms IS NOT NEW.expires_ms OR OLD.original_bytes IS NOT NEW.original_bytes
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER recent_responses_advances_change_cursor_on_delete AFTER DELETE ON recent_responses
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TABLE request_passthrough_v79 (
 request_id TEXT PRIMARY KEY REFERENCES request_attempts(request_id) ON DELETE CASCADE,
 delivered_at_ms INTEGER NOT NULL CHECK (typeof(delivered_at_ms) = 'integer' AND delivered_at_ms BETWEEN 1 AND 9007199254740991),
 state TEXT NOT NULL CHECK (state IN ('pending','bound','answered','no_answer')),
 bound_session TEXT CHECK (bound_session IS NULL OR length(bound_session) > 0),
 bound_turn TEXT CHECK (bound_turn IS NULL OR length(bound_turn) > 0),
 bound_at_ms INTEGER CHECK (bound_at_ms IS NULL OR (typeof(bound_at_ms) = 'integer' AND bound_at_ms BETWEEN 1 AND 9007199254740991)),
 answer_text TEXT CHECK (answer_text IS NULL OR length(CAST(answer_text AS BLOB)) BETWEEN 1 AND 8192),
 answer_truncated INTEGER CHECK (answer_truncated IS NULL OR answer_truncated IN (0,1)),
 answer_provider TEXT CHECK (answer_provider IS NULL OR answer_provider IN ('agy','claude','codex')),
 answer_ended_ms INTEGER CHECK (answer_ended_ms IS NULL OR (typeof(answer_ended_ms) = 'integer' AND answer_ended_ms BETWEEN 1 AND 9007199254740991)),
 settled_at_ms INTEGER CHECK (settled_at_ms IS NULL OR (typeof(settled_at_ms) = 'integer' AND settled_at_ms BETWEEN 1 AND 9007199254740991)),
 folded INTEGER NOT NULL DEFAULT 0 CHECK (folded IN (0, 1) AND (folded = 0 OR state <> 'pending')),
 answer_response INTEGER CHECK (answer_response IS NULL OR (typeof(answer_response) = 'integer' AND answer_response >= 1 AND state = 'answered')),
 CHECK (
  (state = 'pending' AND bound_session IS NULL AND bound_turn IS NULL AND bound_at_ms IS NULL
    AND answer_text IS NULL AND answer_truncated IS NULL AND answer_provider IS NULL AND answer_ended_ms IS NULL AND settled_at_ms IS NULL)
  OR (state = 'bound' AND bound_session IS NOT NULL AND bound_at_ms IS NOT NULL
    AND answer_text IS NULL AND answer_truncated IS NULL AND answer_provider IS NULL AND answer_ended_ms IS NULL AND settled_at_ms IS NULL)
  OR (state = 'answered' AND bound_session IS NOT NULL AND bound_at_ms IS NOT NULL
    AND answer_text IS NOT NULL AND answer_truncated IS NOT NULL AND answer_provider IS NOT NULL AND answer_ended_ms IS NOT NULL AND settled_at_ms IS NOT NULL)
  OR (state = 'no_answer' AND answer_text IS NULL AND answer_truncated IS NULL AND answer_provider IS NULL AND answer_ended_ms IS NULL AND settled_at_ms IS NOT NULL)
 )
);
INSERT INTO request_passthrough_v79(request_id,delivered_at_ms,state,bound_session,bound_turn,bound_at_ms,answer_text,answer_truncated,answer_provider,answer_ended_ms,settled_at_ms,folded,answer_response)
 SELECT request_id,delivered_at_ms,state,bound_session,bound_turn,bound_at_ms,answer_text,answer_truncated,answer_provider,answer_ended_ms,settled_at_ms,folded,answer_response FROM request_passthrough;
DROP TABLE request_passthrough;
ALTER TABLE request_passthrough_v79 RENAME TO request_passthrough;
CREATE INDEX request_passthrough_open ON request_passthrough(state, delivered_at_ms) WHERE state IN ('pending','bound');
CREATE TRIGGER request_passthrough_advances_change_cursor_on_insert AFTER INSERT ON request_passthrough
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE INDEX request_passthrough_response ON request_passthrough(answer_response, delivered_at_ms, request_id) WHERE answer_response IS NOT NULL;
CREATE TRIGGER request_passthrough_advances_change_cursor_on_update AFTER UPDATE ON request_passthrough
WHEN OLD.request_id IS NOT NEW.request_id OR OLD.delivered_at_ms IS NOT NEW.delivered_at_ms OR OLD.state IS NOT NEW.state
  OR OLD.bound_session IS NOT NEW.bound_session OR OLD.bound_turn IS NOT NEW.bound_turn OR OLD.bound_at_ms IS NOT NEW.bound_at_ms
  OR OLD.answer_text IS NOT NEW.answer_text OR OLD.answer_truncated IS NOT NEW.answer_truncated
  OR OLD.answer_provider IS NOT NEW.answer_provider OR OLD.answer_ended_ms IS NOT NEW.answer_ended_ms
  OR OLD.settled_at_ms IS NOT NEW.settled_at_ms OR OLD.folded IS NOT NEW.folded OR OLD.answer_response IS NOT NEW.answer_response
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_passthrough_advances_change_cursor_on_delete AFTER DELETE ON request_passthrough
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;

CREATE INDEX request_passthrough_answered
    ON request_passthrough(settled_at_ms DESC, request_id DESC)
    WHERE state = 'answered';
