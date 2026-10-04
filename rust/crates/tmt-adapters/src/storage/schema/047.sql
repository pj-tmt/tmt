CREATE INDEX request_history_originator_results
    ON request_attempts(originator_identity_id, response_submitted_at_ms DESC, request_id DESC)
    WHERE response_submitted_at_ms IS NOT NULL;
