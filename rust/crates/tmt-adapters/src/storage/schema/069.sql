-- A passthrough typed into a turn that was already running is folded into it (#4912): it is bound
-- to that turn when its text is typed, and the turn's final message answers it. 1 marks the fold,
-- so a read can say the answer is shared with the message that started the turn. A pending
-- capture was matched to no turn and is never folded.
ALTER TABLE request_passthrough ADD COLUMN folded INTEGER NOT NULL DEFAULT 0
 CHECK (folded IN (0, 1) AND (folded = 0 OR state <> 'pending'));
-- A fold is a change like any other: readers waiting on the cursor must wake.
DROP TRIGGER request_passthrough_advances_change_cursor_on_update;
CREATE TRIGGER request_passthrough_advances_change_cursor_on_update AFTER UPDATE ON request_passthrough
WHEN OLD.request_id IS NOT NEW.request_id OR OLD.delivered_at_ms IS NOT NEW.delivered_at_ms OR OLD.state IS NOT NEW.state
  OR OLD.bound_session IS NOT NEW.bound_session OR OLD.bound_turn IS NOT NEW.bound_turn OR OLD.bound_at_ms IS NOT NEW.bound_at_ms
  OR OLD.answer_text IS NOT NEW.answer_text OR OLD.answer_truncated IS NOT NEW.answer_truncated
  OR OLD.answer_provider IS NOT NEW.answer_provider OR OLD.answer_ended_ms IS NOT NEW.answer_ended_ms
  OR OLD.settled_at_ms IS NOT NEW.settled_at_ms OR OLD.folded IS NOT NEW.folded
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
