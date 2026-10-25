-- Explicit input batches and committed input/final order. Bodies remain in
-- their existing request/response owners; these rows grant no runtime authority.
CREATE TABLE capture_batches (
 id TEXT PRIMARY KEY NOT NULL,
 identity_id TEXT NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
 binding_id TEXT NOT NULL,
 driver TEXT NOT NULL,
 session_id TEXT NOT NULL,
 runtime_pid INTEGER NOT NULL CHECK(runtime_pid BETWEEN 1 AND 9007199254740991),
 runtime_start TEXT NOT NULL,
 provider_turn TEXT,
 created_ms INTEGER NOT NULL CHECK(created_ms BETWEEN 0 AND 9007199254740991),
 closed_ms INTEGER CHECK(closed_ms IS NULL OR closed_ms BETWEEN 0 AND 9007199254740991)
) STRICT;
CREATE UNIQUE INDEX capture_batches_current ON capture_batches(identity_id) WHERE closed_ms IS NULL;
CREATE TABLE capture_entries (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT CHECK(sequence BETWEEN 1 AND 9007199254740991),
 identity_id TEXT NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
 batch_id TEXT REFERENCES capture_batches(id) ON DELETE CASCADE,
 kind TEXT NOT NULL CHECK(kind IN ('input','final','reply','failure')),
 request_id TEXT,
 source TEXT NOT NULL DEFAULT 'incoming' CHECK(source IN ('incoming','result','timeout')),
 delivery TEXT NOT NULL DEFAULT 'delivered' CHECK(delivery IN ('pending','delivered','uncertain')),
 checklist_id TEXT,
 response_sequence INTEGER,
 failure TEXT CHECK(failure IN ('worker_error','budget_timeout','storage_error')),
 created_ms INTEGER NOT NULL CHECK(created_ms BETWEEN 0 AND 9007199254740991),
 CHECK((kind IN ('input','reply') AND request_id IS NOT NULL) OR
       (kind='final' AND response_sequence IS NOT NULL) OR
       (kind='failure' AND failure IS NOT NULL))
) STRICT;
CREATE INDEX capture_entries_identity ON capture_entries(identity_id,sequence);
CREATE UNIQUE INDEX capture_entries_input ON capture_entries(identity_id,request_id,source) WHERE kind='input';
CREATE UNIQUE INDEX capture_entries_final ON capture_entries(response_sequence) WHERE kind='final';
CREATE UNIQUE INDEX capture_entries_reply ON capture_entries(request_id) WHERE kind='reply';
CREATE INDEX capture_entries_request ON capture_entries(request_id,kind);
CREATE TRIGGER capture_batches_advances_change_cursor_on_insert AFTER INSERT ON capture_batches
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
CREATE TRIGGER capture_batches_advances_change_cursor_on_delete AFTER DELETE ON capture_batches
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
CREATE TRIGGER capture_batches_advances_change_cursor_on_update AFTER UPDATE ON capture_batches
WHEN OLD.id IS NOT NEW.id OR OLD.identity_id IS NOT NEW.identity_id OR OLD.binding_id IS NOT NEW.binding_id OR OLD.driver IS NOT NEW.driver OR OLD.session_id IS NOT NEW.session_id OR OLD.runtime_pid IS NOT NEW.runtime_pid OR OLD.runtime_start IS NOT NEW.runtime_start OR OLD.provider_turn IS NOT NEW.provider_turn OR OLD.created_ms IS NOT NEW.created_ms OR OLD.closed_ms IS NOT NEW.closed_ms
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
CREATE TRIGGER capture_entries_advances_change_cursor_on_insert AFTER INSERT ON capture_entries
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
CREATE TRIGGER capture_entries_advances_change_cursor_on_delete AFTER DELETE ON capture_entries
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
CREATE TRIGGER capture_entries_advances_change_cursor_on_update AFTER UPDATE ON capture_entries
WHEN OLD.sequence IS NOT NEW.sequence OR OLD.identity_id IS NOT NEW.identity_id OR OLD.batch_id IS NOT NEW.batch_id OR OLD.kind IS NOT NEW.kind OR OLD.request_id IS NOT NEW.request_id OR OLD.source IS NOT NEW.source OR OLD.delivery IS NOT NEW.delivery OR OLD.checklist_id IS NOT NEW.checklist_id OR OLD.response_sequence IS NOT NEW.response_sequence OR OLD.failure IS NOT NEW.failure OR OLD.created_ms IS NOT NEW.created_ms
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;

-- Frozen originator-side acknowledgement keys for the body actually captured.
CREATE TABLE capture_acknowledgements (
 entry_sequence INTEGER NOT NULL REFERENCES capture_entries(sequence) ON DELETE CASCADE,
 request_id TEXT NOT NULL,
 identity_id TEXT NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
 revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 9007199254740991),
 PRIMARY KEY(entry_sequence,request_id)
) STRICT;
CREATE TRIGGER capture_acknowledgements_advances_change_cursor_on_insert AFTER INSERT ON capture_acknowledgements
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
CREATE TRIGGER capture_acknowledgements_advances_change_cursor_on_delete AFTER DELETE ON capture_acknowledgements
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
CREATE TRIGGER capture_acknowledgements_advances_change_cursor_on_update AFTER UPDATE ON capture_acknowledgements
WHEN OLD.entry_sequence IS NOT NEW.entry_sequence OR OLD.request_id IS NOT NEW.request_id OR OLD.identity_id IS NOT NEW.identity_id OR OLD.revision IS NOT NEW.revision
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;

-- Historical finals already have an exact capture order; no input/time links
-- are invented for them.
INSERT INTO capture_entries(identity_id,kind,response_sequence,created_ms)
 SELECT identity_id,'final',sequence,ended_ms FROM recent_responses ORDER BY sequence;

CREATE TABLE capture_floor (
 identity_id TEXT PRIMARY KEY NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
 through_sequence INTEGER NOT NULL CHECK(through_sequence BETWEEN 1 AND 9007199254740991)
) STRICT;
CREATE INDEX capture_entries_expiry ON capture_entries(identity_id,created_ms);
CREATE TRIGGER capture_floor_advances_change_cursor_on_insert AFTER INSERT ON capture_floor
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
CREATE TRIGGER capture_floor_advances_change_cursor_on_delete AFTER DELETE ON capture_floor
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
CREATE TRIGGER capture_floor_advances_change_cursor_on_update AFTER UPDATE ON capture_floor
WHEN OLD.identity_id IS NOT NEW.identity_id OR OLD.through_sequence IS NOT NEW.through_sequence
BEGIN UPDATE change_cursor SET value=value+1 WHERE id=1; END;
