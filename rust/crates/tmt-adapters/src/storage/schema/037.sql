-- Driver-owned resume details beside the remembered session (#420). The session
-- and harness stay first-class columns because core correlates hooks on them;
-- the driver document is versioned by the harness's driver and opaque to core.
ALTER TABLE identity_session_preferences ADD COLUMN driver_state TEXT
  CHECK (driver_state IS NULL OR (typeof(driver_state) = 'text'
    AND length(CAST(driver_state AS BLOB)) BETWEEN 1 AND 1024));
ALTER TABLE identity_session_preferences ADD COLUMN driver_state_version INTEGER
  CHECK (driver_state_version IS NULL OR (typeof(driver_state_version) = 'integer'
    AND driver_state_version BETWEEN 1 AND 65535))
  CHECK ((driver_state IS NULL) = (driver_state_version IS NULL))
  CHECK (driver_state IS NULL OR remembered_harness IS NOT NULL);
-- Set when a resume found the provider session gone; cleared by a new session.
ALTER TABLE identity_session_preferences ADD COLUMN stale_at_ms INTEGER
  CHECK (stale_at_ms IS NULL OR (typeof(stale_at_ms) = 'integer'
    AND stale_at_ms BETWEEN 0 AND 9007199254740991 AND remembered_harness IS NOT NULL));
