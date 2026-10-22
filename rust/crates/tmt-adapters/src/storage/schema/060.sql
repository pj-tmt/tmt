-- The turn output a person's passthrough message earned (#4336). The typed text carries no
-- request id, so a turn is bound to its passthrough by the prompt the provider reported at
-- turn start (migration 059 made a passthrough a mode of the delivery policy). One row per
-- passthrough, created when its text is in the pane; it rides the request through the cascade.
CREATE TABLE request_passthrough (
 request_id TEXT PRIMARY KEY REFERENCES request_attempts(request_id) ON DELETE CASCADE,
 delivered_at_ms INTEGER NOT NULL CHECK (typeof(delivered_at_ms) = 'integer' AND delivered_at_ms BETWEEN 1 AND 9007199254740991),
 state TEXT NOT NULL CHECK (state IN ('pending','bound','answered','no_answer')),
 bound_session TEXT CHECK (bound_session IS NULL OR length(bound_session) > 0),
 bound_turn TEXT CHECK (bound_turn IS NULL OR length(bound_turn) > 0),
 bound_at_ms INTEGER CHECK (bound_at_ms IS NULL OR (typeof(bound_at_ms) = 'integer' AND bound_at_ms BETWEEN 1 AND 9007199254740991)),
 answer_text TEXT CHECK (answer_text IS NULL OR length(CAST(answer_text AS BLOB)) BETWEEN 1 AND 8192),
 answer_truncated INTEGER CHECK (answer_truncated IS NULL OR answer_truncated IN (0,1)),
 answer_provider TEXT CHECK (answer_provider IS NULL OR answer_provider IN ('claude','codex')),
 answer_ended_ms INTEGER CHECK (answer_ended_ms IS NULL OR (typeof(answer_ended_ms) = 'integer' AND answer_ended_ms BETWEEN 1 AND 9007199254740991)),
 settled_at_ms INTEGER CHECK (settled_at_ms IS NULL OR (typeof(settled_at_ms) = 'integer' AND settled_at_ms BETWEEN 1 AND 9007199254740991)),
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
CREATE INDEX request_passthrough_open ON request_passthrough(state, delivered_at_ms) WHERE state IN ('pending','bound');
CREATE TRIGGER request_passthrough_advances_change_cursor_on_insert AFTER INSERT ON request_passthrough
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_passthrough_advances_change_cursor_on_update AFTER UPDATE ON request_passthrough
WHEN OLD.request_id IS NOT NEW.request_id OR OLD.delivered_at_ms IS NOT NEW.delivered_at_ms OR OLD.state IS NOT NEW.state
  OR OLD.bound_session IS NOT NEW.bound_session OR OLD.bound_turn IS NOT NEW.bound_turn OR OLD.bound_at_ms IS NOT NEW.bound_at_ms
  OR OLD.answer_text IS NOT NEW.answer_text OR OLD.answer_truncated IS NOT NEW.answer_truncated
  OR OLD.answer_provider IS NOT NEW.answer_provider OR OLD.answer_ended_ms IS NOT NEW.answer_ended_ms
  OR OLD.settled_at_ms IS NOT NEW.settled_at_ms
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_passthrough_advances_change_cursor_on_delete AFTER DELETE ON request_passthrough
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
