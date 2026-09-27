-- A launched child remains addressable only while its foreground owner survives.
-- Hook-admitted runtimes have no launch owner and retain their existing contract.
ALTER TABLE bindings ADD COLUMN launch_owner_pid INTEGER
  CHECK (launch_owner_pid > 0 AND launch_owner_pid <= 9007199254740991);
ALTER TABLE bindings ADD COLUMN launch_owner_start_identity TEXT
  CHECK ((launch_owner_pid IS NULL AND launch_owner_start_identity IS NULL)
    OR (launch_owner_pid IS NOT NULL AND launch_owner_start_identity IS NOT NULL AND runtime_pid IS NOT NULL));
