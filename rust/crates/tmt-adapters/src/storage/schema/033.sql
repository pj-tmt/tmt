ALTER TABLE bindings ADD COLUMN runtime_state TEXT NOT NULL DEFAULT 'unknown'
  CHECK (runtime_state IN ('unknown', 'running', 'ended'));
ALTER TABLE bindings ADD COLUMN last_transition TEXT
  CHECK (last_transition IN ('started', 'resumed', 'cleared', 'compacted', 'forked', 'ended'));
ALTER TABLE bindings ADD COLUMN runtime_pid INTEGER CHECK (runtime_pid > 0 AND runtime_pid <= 9007199254740991);
ALTER TABLE bindings ADD COLUMN runtime_start_identity TEXT;
ALTER TABLE bindings ADD COLUMN observed_provider_session_id TEXT
  CHECK ((runtime_pid IS NULL AND runtime_start_identity IS NULL AND observed_provider_session_id IS NULL)
    OR (runtime_pid IS NOT NULL AND runtime_start_identity IS NOT NULL))
  CHECK (runtime_state = 'unknown' OR runtime_pid IS NOT NULL);

-- Preferences belong to the identity, not to a disposable interface binding.
-- Retiring an identity hides this retained record through the active-identity reader.
CREATE TABLE identity_session_preferences (
  identity_id TEXT NOT NULL PRIMARY KEY REFERENCES identities(id) ON DELETE CASCADE,
  preferred_harness TEXT,
  remembered_harness TEXT,
  runtime_mode TEXT,
  provider_session_id TEXT,
  CHECK ((remembered_harness IS NULL AND runtime_mode IS NULL AND provider_session_id IS NULL)
    OR (remembered_harness IS NOT NULL AND runtime_mode IS NOT NULL AND provider_session_id IS NOT NULL))
);
