-- A resume launch marks its session pending until a provider start confirms
-- it (#420). Only an unconfirmed, trustworthy failure becomes stale_at_ms, so a
-- crashed launcher leaves this harmless mark rather than a false stale one.
ALTER TABLE identity_session_preferences ADD COLUMN resume_pending_at_ms INTEGER
  CHECK (resume_pending_at_ms IS NULL OR (typeof(resume_pending_at_ms) = 'integer'
    AND resume_pending_at_ms BETWEEN 0 AND 9007199254740991 AND remembered_harness IS NOT NULL));
