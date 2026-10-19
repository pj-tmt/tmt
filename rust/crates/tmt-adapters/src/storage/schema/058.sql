-- Codex sessions a bound session started (subagents and auto-review) count toward the seat
-- that owns the root of their parent chain. One family per consumption source and one row
-- per child rollout still being followed: the sampler is a fresh process each tick, so the
-- per-file cursors cannot live in the 1 KiB driver state. Rows hold counters and cursors,
-- never transcript content. A family is deleted with its source, and replaced when the
-- source's session changes.
CREATE TABLE consumption_family (
 identity_id TEXT PRIMARY KEY REFERENCES consumption_sources(identity_id) ON DELETE CASCADE,
 session TEXT NOT NULL CHECK (length(session) BETWEEN 1 AND 256),
 scan TEXT CHECK (scan IS NULL OR length(CAST(scan AS BLOB)) BETWEEN 1 AND 64),
 edge TEXT NOT NULL CHECK (length(CAST(edge AS BLOB)) BETWEEN 2 AND 8192),
 closed TEXT NOT NULL CHECK (length(CAST(closed AS BLOB)) BETWEEN 2 AND 256),
 warm INTEGER NOT NULL CHECK (warm IN (0,1))
);
CREATE TABLE consumption_family_files (
 identity_id TEXT NOT NULL REFERENCES consumption_family(identity_id) ON DELETE CASCADE,
 file TEXT NOT NULL CHECK (length(CAST(file AS BLOB)) BETWEEN 1 AND 4096),
 child TEXT NOT NULL CHECK (length(child) BETWEEN 1 AND 256),
 parent TEXT NOT NULL CHECK (length(parent) BETWEEN 1 AND 256),
 base TEXT NOT NULL CHECK (length(CAST(base AS BLOB)) BETWEEN 2 AND 256),
 state TEXT NOT NULL CHECK (length(CAST(state AS BLOB)) BETWEEN 2 AND 2048),
 touched_ms INTEGER NOT NULL CHECK (typeof(touched_ms) = 'integer' AND touched_ms > 0),
 PRIMARY KEY(identity_id, file)
);
CREATE TRIGGER consumption_family_advances_change_cursor_on_insert AFTER INSERT ON consumption_family
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER consumption_family_advances_change_cursor_on_update AFTER UPDATE ON consumption_family
WHEN OLD.identity_id IS NOT NEW.identity_id OR OLD.session IS NOT NEW.session OR OLD.scan IS NOT NEW.scan OR OLD.edge IS NOT NEW.edge OR OLD.closed IS NOT NEW.closed OR OLD.warm IS NOT NEW.warm
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER consumption_family_advances_change_cursor_on_delete AFTER DELETE ON consumption_family
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER consumption_family_files_advances_change_cursor_on_insert AFTER INSERT ON consumption_family_files
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER consumption_family_files_advances_change_cursor_on_update AFTER UPDATE ON consumption_family_files
WHEN OLD.identity_id IS NOT NEW.identity_id OR OLD.file IS NOT NEW.file OR OLD.child IS NOT NEW.child OR OLD.parent IS NOT NEW.parent OR OLD.base IS NOT NEW.base OR OLD.state IS NOT NEW.state OR OLD.touched_ms IS NOT NEW.touched_ms
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER consumption_family_files_advances_change_cursor_on_delete AFTER DELETE ON consumption_family_files
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
