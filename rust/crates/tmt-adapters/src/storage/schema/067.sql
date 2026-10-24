-- A launch rendezvous records intent/evidence only, never delivery authority.
CREATE TABLE launch_observations (
  binding_id TEXT NOT NULL REFERENCES bindings(id) ON DELETE CASCADE,
  owner_pid INTEGER NOT NULL CHECK (typeof(owner_pid)='integer' AND owner_pid BETWEEN 1 AND 9007199254740991),
  owner_start TEXT NOT NULL CHECK (length(CAST(owner_start AS BLOB)) BETWEEN 1 AND 256),
  driver TEXT NOT NULL CHECK (length(driver) BETWEEN 1 AND 64 AND driver GLOB '[a-z]*' AND driver NOT GLOB '*[^a-z0-9_-]*'),
  created_at_ms INTEGER NOT NULL CHECK (typeof(created_at_ms)='integer' AND created_at_ms BETWEEN 0 AND 9007199254740991),
  expires_at_ms INTEGER NOT NULL CHECK (typeof(expires_at_ms)='integer' AND expires_at_ms BETWEEN created_at_ms AND 9007199254740991),
  status TEXT NOT NULL DEFAULT 'intent' CHECK (status IN ('intent','pending','admitted','settled')),
  provider_pid INTEGER CHECK (provider_pid IS NULL OR (typeof(provider_pid)='integer' AND provider_pid BETWEEN 1 AND 9007199254740991)),
  provider_start TEXT CHECK (provider_start IS NULL OR length(CAST(provider_start AS BLOB)) BETWEEN 1 AND 256),
  provider_session TEXT CHECK (provider_session IS NULL OR length(CAST(provider_session AS BLOB)) BETWEEN 1 AND 256),
  runtime_mode TEXT CHECK (runtime_mode IS NULL OR (length(runtime_mode) BETWEEN 1 AND 64 AND runtime_mode GLOB '[a-z]*' AND runtime_mode NOT GLOB '*[^a-z0-9_-]*')),
  driver_state_version INTEGER,
  driver_state TEXT CHECK (driver_state IS NULL OR (driver_state_version BETWEEN 1 AND 65535 AND length(CAST(driver_state AS BLOB)) BETWEEN 1 AND 1024)),
  observed_at_ms INTEGER CHECK (observed_at_ms IS NULL OR (typeof(observed_at_ms)='integer' AND observed_at_ms BETWEEN created_at_ms AND expires_at_ms)),
  step TEXT CHECK (step IS NULL OR step IN ('input','decode','driver_admission','host_discovery','pane_snapshot','provider_verification','storage','context')),
  reason TEXT CHECK (reason IS NULL OR reason IN ('not_admitted','worker_error','budget_timeout','spawn_failed','launch_ended','conflict')),
  CHECK ((driver_state IS NULL) = (driver_state_version IS NULL)),
  CHECK ((step IS NULL) = (reason IS NULL)),
  CHECK ((status='pending') = (provider_pid IS NOT NULL)),
  PRIMARY KEY (binding_id,owner_pid,owner_start),
  CHECK ((provider_pid IS NULL) = (provider_start IS NULL)
     AND (provider_pid IS NULL) = (provider_session IS NULL)
     AND (provider_pid IS NULL) = (runtime_mode IS NULL)
     AND (provider_pid IS NULL) = (observed_at_ms IS NULL))
);
CREATE INDEX launch_observations_expiry ON launch_observations(expires_at_ms);
CREATE TRIGGER launch_observations_advances_change_cursor_on_insert
AFTER INSERT ON launch_observations
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER launch_observations_advances_change_cursor_on_update
AFTER UPDATE ON launch_observations
WHEN OLD.binding_id IS NOT NEW.binding_id
 OR OLD.owner_pid IS NOT NEW.owner_pid
 OR OLD.owner_start IS NOT NEW.owner_start
 OR OLD.driver IS NOT NEW.driver
 OR OLD.created_at_ms IS NOT NEW.created_at_ms
 OR OLD.expires_at_ms IS NOT NEW.expires_at_ms
 OR OLD.status IS NOT NEW.status
 OR OLD.provider_pid IS NOT NEW.provider_pid
 OR OLD.provider_start IS NOT NEW.provider_start
 OR OLD.provider_session IS NOT NEW.provider_session
 OR OLD.runtime_mode IS NOT NEW.runtime_mode
 OR OLD.driver_state IS NOT NEW.driver_state
 OR OLD.driver_state_version IS NOT NEW.driver_state_version
 OR OLD.observed_at_ms IS NOT NEW.observed_at_ms
 OR OLD.step IS NOT NEW.step
 OR OLD.reason IS NOT NEW.reason
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER launch_observations_advances_change_cursor_on_delete
AFTER DELETE ON launch_observations
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
