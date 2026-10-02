-- Advisory reply notices retain only rendered hints, not final bodies or receipts.
CREATE TABLE reply_notice_batches (
 id TEXT PRIMARY KEY NOT NULL,
 originator_id TEXT NOT NULL REFERENCES identities(id),
 binding_id TEXT NOT NULL,
 due_ms INTEGER NOT NULL CHECK (due_ms > 0),
 window_ms INTEGER NOT NULL CHECK (window_ms BETWEEN 0 AND 60000),
 quiet_ms INTEGER NOT NULL CHECK (quiet_ms BETWEEN 0 AND 30000),
 worker_pid INTEGER,
 worker_start TEXT,
 sending INTEGER NOT NULL DEFAULT 0 CHECK (sending IN (0, 1)),
 CHECK (sending=0 OR worker_pid IS NOT NULL),
 CHECK ((worker_pid IS NULL) = (worker_start IS NULL)),
 CHECK (worker_pid IS NULL OR (worker_pid BETWEEN 1 AND 9007199254740991 AND length(worker_start) BETWEEN 1 AND 256 AND trim(worker_start) <> '' AND worker_start NOT GLOB '*[^ -~]*'))
);
CREATE INDEX reply_notice_batches_pending ON reply_notice_batches(binding_id, sending);
CREATE UNIQUE INDEX reply_notice_batches_one_sender ON reply_notice_batches(binding_id) WHERE sending=1;
CREATE TABLE reply_notices (
 request_id TEXT PRIMARY KEY NOT NULL REFERENCES request_notifications(request_id) ON DELETE CASCADE,
 batch_id TEXT NOT NULL REFERENCES reply_notice_batches(id) ON DELETE CASCADE,
 text TEXT NOT NULL CHECK (length(CAST(text AS BLOB)) <= 1024),
 attempted INTEGER NOT NULL DEFAULT 0 CHECK (attempted IN (0, 1))
);
CREATE INDEX reply_notices_batch ON reply_notices(batch_id, request_id);
CREATE TRIGGER reply_notice_batches_advances_change_cursor_on_insert AFTER INSERT ON reply_notice_batches
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER reply_notice_batches_advances_change_cursor_on_delete AFTER DELETE ON reply_notice_batches
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER reply_notice_batches_advances_change_cursor_on_update AFTER UPDATE ON reply_notice_batches
WHEN OLD.id IS NOT NEW.id OR OLD.originator_id IS NOT NEW.originator_id OR OLD.binding_id IS NOT NEW.binding_id OR OLD.due_ms IS NOT NEW.due_ms OR OLD.window_ms IS NOT NEW.window_ms OR OLD.quiet_ms IS NOT NEW.quiet_ms OR OLD.worker_pid IS NOT NEW.worker_pid OR OLD.worker_start IS NOT NEW.worker_start OR OLD.sending IS NOT NEW.sending
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER reply_notices_advances_change_cursor_on_insert AFTER INSERT ON reply_notices
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER reply_notices_advances_change_cursor_on_delete AFTER DELETE ON reply_notices
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER reply_notices_advances_change_cursor_on_update AFTER UPDATE ON reply_notices
WHEN OLD.request_id IS NOT NEW.request_id OR OLD.batch_id IS NOT NEW.batch_id OR OLD.text IS NOT NEW.text OR OLD.attempted IS NOT NEW.attempted
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
