-- The retained final a captured passthrough answer was taken from (#4993). The capture keeps its
-- own copy of the text; this is the `recent_responses` sequence, so one final has one identity in
-- both places. It is set in the transaction that inserts the final and settles the capture, and
-- stays when the final is pruned (the number is never reused). An answer settled earlier has none.
ALTER TABLE request_passthrough ADD COLUMN answer_response INTEGER
 CHECK (answer_response IS NULL OR (typeof(answer_response) = 'integer' AND answer_response >= 1 AND state = 'answered'));
-- A response can answer several folded texts: the read finds the oldest by this index.
CREATE INDEX request_passthrough_response ON request_passthrough(answer_response, delivered_at_ms, request_id) WHERE answer_response IS NOT NULL;
DROP TRIGGER request_passthrough_advances_change_cursor_on_update;
CREATE TRIGGER request_passthrough_advances_change_cursor_on_update AFTER UPDATE ON request_passthrough
WHEN OLD.request_id IS NOT NEW.request_id OR OLD.delivered_at_ms IS NOT NEW.delivered_at_ms OR OLD.state IS NOT NEW.state
  OR OLD.bound_session IS NOT NEW.bound_session OR OLD.bound_turn IS NOT NEW.bound_turn OR OLD.bound_at_ms IS NOT NEW.bound_at_ms
  OR OLD.answer_text IS NOT NEW.answer_text OR OLD.answer_truncated IS NOT NEW.answer_truncated
  OR OLD.answer_provider IS NOT NEW.answer_provider OR OLD.answer_ended_ms IS NOT NEW.answer_ended_ms
  OR OLD.settled_at_ms IS NOT NEW.settled_at_ms OR OLD.folded IS NOT NEW.folded OR OLD.answer_response IS NOT NEW.answer_response
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
