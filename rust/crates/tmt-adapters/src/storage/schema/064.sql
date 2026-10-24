-- The turns a person's passthrough messages have answered, newest first (#4685). A turn's
-- originator is on its attempt, so the answers read walks this index and tests each entry.
CREATE INDEX request_passthrough_answered
    ON request_passthrough(settled_at_ms DESC, request_id DESC)
    WHERE state = 'answered';
