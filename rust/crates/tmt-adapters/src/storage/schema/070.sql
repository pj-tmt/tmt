-- The first turn that began after a request was handed to its recipient (#4911). The recipient's
-- activity alone cannot say that turn has ended (Claude names no turn, and a phase change inside
-- one turn moves its start), so the hook transaction that admits a main-session turn boundary
-- records it: `started_ms` when the turn began, `ended_ms` when it ended. One row per request; it
-- rides the request through the cascade. Requests handed over before this migration have none.
CREATE TABLE request_turns (
 request_id TEXT PRIMARY KEY REFERENCES request_attempts(request_id) ON DELETE CASCADE,
 started_ms INTEGER NOT NULL CHECK (typeof(started_ms) = 'integer' AND started_ms BETWEEN 1 AND 9007199254740991),
 ended_ms INTEGER CHECK (ended_ms IS NULL OR (typeof(ended_ms) = 'integer' AND ended_ms BETWEEN started_ms AND 9007199254740991))
);
-- A message's state follows its turn, so readers waiting on the cursor must wake.
CREATE TRIGGER request_turns_advances_change_cursor_on_insert AFTER INSERT ON request_turns
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_turns_advances_change_cursor_on_update AFTER UPDATE ON request_turns
WHEN OLD.request_id IS NOT NEW.request_id OR OLD.started_ms IS NOT NEW.started_ms OR OLD.ended_ms IS NOT NEW.ended_ms
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER request_turns_advances_change_cursor_on_delete AFTER DELETE ON request_turns
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
