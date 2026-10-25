-- Orchestration slice 1: policy data and append-only explanations, without live producers.
CREATE TABLE orchestration_settings (
 id INTEGER PRIMARY KEY CHECK (id = 1),
 revision INTEGER NOT NULL CHECK (typeof(revision) = 'integer' AND revision BETWEEN 1 AND 9007199254740991),
 time_zone TEXT CHECK (time_zone IS NULL OR length(time_zone) BETWEEN 1 AND 128)
);
CREATE TRIGGER orchestration_settings_advances_change_cursor_on_insert AFTER INSERT ON orchestration_settings
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER orchestration_settings_advances_change_cursor_on_update AFTER UPDATE ON orchestration_settings
WHEN OLD.id IS NOT NEW.id OR OLD.revision IS NOT NEW.revision OR OLD.time_zone IS NOT NEW.time_zone
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER orchestration_settings_advances_change_cursor_on_delete AFTER DELETE ON orchestration_settings
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TABLE orchestration_plans (
 id TEXT NOT NULL PRIMARY KEY,
 name TEXT NOT NULL CHECK (length(CAST(name AS BLOB)) BETWEEN 1 AND 80),
 revision INTEGER NOT NULL CHECK (typeof(revision) = 'integer' AND revision BETWEEN 1 AND 9007199254740991),
 priority INTEGER NOT NULL UNIQUE CHECK (typeof(priority) = 'integer' AND priority BETWEEN -2147483648 AND 2147483647),
 enabled INTEGER NOT NULL CHECK (typeof(enabled) = 'integer' AND enabled IN (0,1)),
 schedule TEXT NOT NULL CHECK (json_valid(schedule) AND length(CAST(schedule AS BLOB)) <= 32768),
 fields TEXT NOT NULL CHECK (json_valid(fields) AND length(CAST(fields AS BLOB)) <= 32768)
);
CREATE TRIGGER orchestration_plans_advances_change_cursor_on_insert AFTER INSERT ON orchestration_plans
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER orchestration_plans_advances_change_cursor_on_update AFTER UPDATE ON orchestration_plans
WHEN OLD.id IS NOT NEW.id OR OLD.name IS NOT NEW.name OR OLD.revision IS NOT NEW.revision OR OLD.priority IS NOT NEW.priority OR OLD.enabled IS NOT NEW.enabled OR OLD.schedule IS NOT NEW.schedule OR OLD.fields IS NOT NEW.fields
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER orchestration_plans_advances_change_cursor_on_delete AFTER DELETE ON orchestration_plans
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TABLE orchestration_plan_targets (
 plan_id TEXT NOT NULL REFERENCES orchestration_plans(id) ON DELETE CASCADE,
 position INTEGER NOT NULL CHECK (typeof(position) = 'integer' AND position BETWEEN 0 AND 255),
 target TEXT NOT NULL CHECK (json_valid(target) AND length(CAST(target AS BLOB)) <= 512),
 PRIMARY KEY(plan_id,position)
);
CREATE TRIGGER orchestration_plan_targets_advances_change_cursor_on_insert AFTER INSERT ON orchestration_plan_targets
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER orchestration_plan_targets_advances_change_cursor_on_update AFTER UPDATE ON orchestration_plan_targets
WHEN OLD.plan_id IS NOT NEW.plan_id OR OLD.position IS NOT NEW.position OR OLD.target IS NOT NEW.target
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER orchestration_plan_targets_advances_change_cursor_on_delete AFTER DELETE ON orchestration_plan_targets
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TABLE orchestration_member_overrides (
 plan_id TEXT NOT NULL REFERENCES orchestration_plans(id) ON DELETE CASCADE,
 identity_id TEXT NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
 fields TEXT NOT NULL CHECK (json_valid(fields) AND length(CAST(fields AS BLOB)) <= 32768),
 PRIMARY KEY(plan_id,identity_id)
);
CREATE TRIGGER orchestration_member_overrides_advances_change_cursor_on_insert AFTER INSERT ON orchestration_member_overrides
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER orchestration_member_overrides_advances_change_cursor_on_update AFTER UPDATE ON orchestration_member_overrides
WHEN OLD.plan_id IS NOT NEW.plan_id OR OLD.identity_id IS NOT NEW.identity_id OR OLD.fields IS NOT NEW.fields
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER orchestration_member_overrides_advances_change_cursor_on_delete AFTER DELETE ON orchestration_member_overrides
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TABLE orchestration_focus (
 identity_id TEXT NOT NULL PRIMARY KEY REFERENCES identities(id) ON DELETE CASCADE,
 revision INTEGER NOT NULL CHECK (typeof(revision) = 'integer' AND revision BETWEEN 1 AND 9007199254740991),
 start_ms INTEGER NOT NULL CHECK (typeof(start_ms) = 'integer' AND start_ms BETWEEN 1 AND 9007199254740991),
 end_ms INTEGER NOT NULL CHECK (typeof(end_ms) = 'integer' AND end_ms > start_ms AND end_ms <= 9007199254740991)
);
CREATE TRIGGER orchestration_focus_advances_change_cursor_on_insert AFTER INSERT ON orchestration_focus
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER orchestration_focus_advances_change_cursor_on_update AFTER UPDATE ON orchestration_focus
WHEN OLD.identity_id IS NOT NEW.identity_id OR OLD.revision IS NOT NEW.revision OR OLD.start_ms IS NOT NEW.start_ms OR OLD.end_ms IS NOT NEW.end_ms
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER orchestration_focus_advances_change_cursor_on_delete AFTER DELETE ON orchestration_focus
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TABLE orchestration_decisions (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 recorded_ms INTEGER NOT NULL CHECK (typeof(recorded_ms) = 'integer' AND recorded_ms BETWEEN 1 AND 9007199254740991),
 expires_ms INTEGER NOT NULL CHECK (typeof(expires_ms) = 'integer' AND expires_ms > recorded_ms AND expires_ms <= 9007199254740991),
 subject_kind TEXT NOT NULL CHECK (subject_kind IN ('request','identity','job')),
 subject_id TEXT NOT NULL CHECK (length(subject_id) BETWEEN 1 AND 128),
 actor_id TEXT,
 recipient_id TEXT,
 action TEXT NOT NULL CHECK (action IN ('block','delay','send','jobRun')),
 outcome TEXT NOT NULL CHECK (outcome IN ('allowed','held','sent','uncertain','failed','completed')),
 reason TEXT NOT NULL CHECK (length(CAST(reason AS BLOB)) BETWEEN 1 AND 256),
 configuration_revision INTEGER NOT NULL CHECK (typeof(configuration_revision) = 'integer' AND configuration_revision BETWEEN 0 AND 9007199254740991),
 time_zone TEXT NOT NULL CHECK (length(time_zone) BETWEEN 1 AND 128),
 sources TEXT NOT NULL CHECK (json_valid(sources) AND length(CAST(sources AS BLOB)) <= 8192)
);
-- Readers page this journal on demand by event ID; appends and pruning do not wake boards.
CREATE INDEX orchestration_decisions_subject ON orchestration_decisions(subject_kind,subject_id,id DESC);
CREATE INDEX orchestration_decisions_expiry ON orchestration_decisions(expires_ms,id);
