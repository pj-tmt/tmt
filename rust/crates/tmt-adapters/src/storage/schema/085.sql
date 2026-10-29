-- Private complete-final identity, computed before the retained text is cut.
-- Old truncated responses cannot establish equality; preserve them unchanged.
ALTER TABLE recent_responses ADD COLUMN content_digest TEXT
 CHECK (content_digest IS NULL OR (length(content_digest)=64 AND content_digest NOT GLOB '*[^0-9a-f]*'));
DROP TRIGGER recent_responses_advances_change_cursor_on_update;
CREATE TRIGGER recent_responses_advances_change_cursor_on_update AFTER UPDATE ON recent_responses
WHEN OLD.sequence IS NOT NEW.sequence OR OLD.identity_id IS NOT NEW.identity_id OR OLD.text IS NOT NEW.text OR OLD.ended_ms IS NOT NEW.ended_ms OR OLD.provider IS NOT NEW.provider OR OLD.truncated IS NOT NEW.truncated OR OLD.session_id IS NOT NEW.session_id OR OLD.expires_ms IS NOT NEW.expires_ms OR OLD.original_bytes IS NOT NEW.original_bytes OR OLD.content_digest IS NOT NEW.content_digest
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
