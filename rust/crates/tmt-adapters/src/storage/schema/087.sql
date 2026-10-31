-- One mutable job owner. Legacy mappings survive removal; display numbers never recycle.
CREATE TABLE orchestration_jobs (
 number INTEGER PRIMARY KEY AUTOINCREMENT CHECK (number BETWEEN 1 AND 9007199254740991),
 id TEXT NOT NULL UNIQUE,
 revision INTEGER NOT NULL CHECK (typeof(revision) = 'integer' AND revision BETWEEN 1 AND 9007199254740991),
 owner_id TEXT REFERENCES identities(id),
 room_id TEXT,
 title TEXT CHECK (title IS NULL OR length(CAST(title AS BLOB)) BETWEEN 1 AND 200),
 message TEXT NOT NULL CHECK (length(CAST(message AS BLOB)) BETWEEN 1 AND 1048576),
 schedule TEXT NOT NULL CHECK (json_valid(schedule) AND length(CAST(schedule AS BLOB)) <= 4096),
 pause_by TEXT,
 pause_ms INTEGER CHECK (pause_ms IS NULL OR (typeof(pause_ms) = 'integer' AND pause_ms BETWEEN -9007199254740991 AND 9007199254740991)),
 pending_revision INTEGER,
 pending_ms INTEGER,
 removed INTEGER NOT NULL DEFAULT 0 CHECK (removed IN (0,1)),
 CHECK ((pause_by IS NULL) = (pause_ms IS NULL)),
 CHECK (owner_id IS NOT NULL OR pause_by IS NOT NULL),
 CHECK ((pending_revision IS NULL) = (pending_ms IS NULL)),
 CHECK (pending_revision IS NULL OR (pending_revision = revision AND typeof(pending_ms) = 'integer' AND pending_ms BETWEEN -9007199254740991 AND 9007199254740991))
);
CREATE INDEX orchestration_jobs_room ON orchestration_jobs(room_id,number) WHERE removed=0;
CREATE INDEX orchestration_jobs_owner ON orchestration_jobs(owner_id,number) WHERE removed=0;
CREATE TABLE orchestration_job_legacy (
 room_id TEXT NOT NULL,
 cron_id INTEGER NOT NULL CHECK (typeof(cron_id) = 'integer' AND cron_id BETWEEN 1 AND 9007199254740991),
 job_id TEXT NOT NULL UNIQUE REFERENCES orchestration_jobs(id),
 PRIMARY KEY(room_id,cron_id)
);
-- Retain counters even for an empty room. Import receipts fence an uncertain cutover.
CREATE TABLE orchestration_job_imports (
 source_id TEXT PRIMARY KEY,
 source_digest TEXT NOT NULL,
 receipt TEXT NOT NULL CHECK (json_valid(receipt) AND length(CAST(receipt AS BLOB)) <= 16777216)
);
CREATE TABLE orchestration_job_legacy_counters (
 room_id TEXT PRIMARY KEY,
 last_id INTEGER NOT NULL CHECK (typeof(last_id) = 'integer' AND last_id BETWEEN 0 AND 9007199254740991)
);
CREATE TABLE orchestration_job_runs (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 job_id TEXT NOT NULL REFERENCES orchestration_jobs(id),
 revision INTEGER NOT NULL CHECK (revision BETWEEN 1 AND 9007199254740991),
 slot_ms INTEGER NOT NULL CHECK (typeof(slot_ms) = 'integer' AND slot_ms BETWEEN -9007199254740991 AND 9007199254740991),
 observed_slot_ms INTEGER NOT NULL CHECK (typeof(observed_slot_ms) = 'integer' AND observed_slot_ms BETWEEN -9007199254740991 AND 9007199254740991),
 manual INTEGER NOT NULL CHECK (manual IN (0,1)),
 operation_id TEXT NOT NULL UNIQUE REFERENCES office_dispatch_operations(operation_id),
 recorded_ms INTEGER NOT NULL CHECK (typeof(recorded_ms) = 'integer' AND recorded_ms BETWEEN 1 AND 9007199254740991)
);
CREATE UNIQUE INDEX orchestration_job_runs_observed_slot ON orchestration_job_runs(job_id,observed_slot_ms) WHERE manual=0;
CREATE INDEX orchestration_job_runs_job ON orchestration_job_runs(job_id,id DESC);
CREATE TRIGGER orchestration_jobs_advances_change_cursor_on_insert AFTER INSERT ON orchestration_jobs
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
CREATE TRIGGER orchestration_jobs_advances_change_cursor_on_update AFTER UPDATE ON orchestration_jobs
WHEN OLD.id IS NOT NEW.id OR OLD.number IS NOT NEW.number OR OLD.revision IS NOT NEW.revision OR OLD.owner_id IS NOT NEW.owner_id OR OLD.room_id IS NOT NEW.room_id
 OR OLD.title IS NOT NEW.title OR OLD.message IS NOT NEW.message OR OLD.schedule IS NOT NEW.schedule
 OR OLD.pause_by IS NOT NEW.pause_by OR OLD.pause_ms IS NOT NEW.pause_ms
 OR OLD.pending_revision IS NOT NEW.pending_revision OR OLD.pending_ms IS NOT NEW.pending_ms OR OLD.removed IS NOT NEW.removed
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
CREATE TRIGGER orchestration_jobs_advances_change_cursor_on_delete AFTER DELETE ON orchestration_jobs
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
CREATE TRIGGER orchestration_job_legacy_advances_change_cursor_on_insert AFTER INSERT ON orchestration_job_legacy
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
CREATE TRIGGER orchestration_job_legacy_advances_change_cursor_on_update AFTER UPDATE ON orchestration_job_legacy
WHEN OLD.room_id IS NOT NEW.room_id OR OLD.cron_id IS NOT NEW.cron_id OR OLD.job_id IS NOT NEW.job_id
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
CREATE TRIGGER orchestration_job_legacy_advances_change_cursor_on_delete AFTER DELETE ON orchestration_job_legacy
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
CREATE TRIGGER orchestration_job_imports_advances_change_cursor_on_insert AFTER INSERT ON orchestration_job_imports
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
CREATE TRIGGER orchestration_job_imports_advances_change_cursor_on_update AFTER UPDATE ON orchestration_job_imports
WHEN OLD.source_id IS NOT NEW.source_id OR OLD.source_digest IS NOT NEW.source_digest OR OLD.receipt IS NOT NEW.receipt
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
CREATE TRIGGER orchestration_job_imports_advances_change_cursor_on_delete AFTER DELETE ON orchestration_job_imports
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
CREATE TRIGGER orchestration_job_legacy_counters_advances_change_cursor_on_insert AFTER INSERT ON orchestration_job_legacy_counters
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
CREATE TRIGGER orchestration_job_legacy_counters_advances_change_cursor_on_update AFTER UPDATE ON orchestration_job_legacy_counters
WHEN OLD.room_id IS NOT NEW.room_id OR OLD.last_id IS NOT NEW.last_id
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
CREATE TRIGGER orchestration_job_legacy_counters_advances_change_cursor_on_delete AFTER DELETE ON orchestration_job_legacy_counters
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
CREATE TRIGGER orchestration_job_runs_advances_change_cursor_on_insert AFTER INSERT ON orchestration_job_runs
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
CREATE TRIGGER orchestration_job_runs_advances_change_cursor_on_update AFTER UPDATE ON orchestration_job_runs
WHEN OLD.id IS NOT NEW.id OR OLD.job_id IS NOT NEW.job_id OR OLD.revision IS NOT NEW.revision OR OLD.slot_ms IS NOT NEW.slot_ms OR OLD.observed_slot_ms IS NOT NEW.observed_slot_ms OR OLD.manual IS NOT NEW.manual OR OLD.operation_id IS NOT NEW.operation_id OR OLD.recorded_ms IS NOT NEW.recorded_ms
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
CREATE TRIGGER orchestration_job_runs_advances_change_cursor_on_delete AFTER DELETE ON orchestration_job_runs
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
