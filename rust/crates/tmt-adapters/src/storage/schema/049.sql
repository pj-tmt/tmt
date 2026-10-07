CREATE TABLE focus_policies (
    identity_id TEXT PRIMARY KEY REFERENCES identities(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL CHECK (revision BETWEEN 1 AND 9007199254740991),
    until_ms INTEGER NOT NULL CHECK (until_ms BETWEEN 0 AND 9007199254740991),
    owner_identity_id TEXT NOT NULL,
    setter_identity_id TEXT NOT NULL
);
CREATE TABLE request_delivery_policies (
    request_id TEXT PRIMARY KEY REFERENCES request_attempts(request_id) ON DELETE CASCADE,
    urgent INTEGER NOT NULL CHECK (urgent IN (0, 1)),
    kind TEXT NOT NULL CHECK (kind IN ('decision', 'review', 'fyi')),
    automatic INTEGER NOT NULL CHECK (automatic IN (0, 1))
);
CREATE TABLE focus_checklists (
    id TEXT PRIMARY KEY,
    identity_id TEXT NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
    attempt_token TEXT NOT NULL UNIQUE,
    through_sequence INTEGER NOT NULL CHECK (through_sequence BETWEEN 1 AND 9007199254740991),
    state TEXT NOT NULL CHECK (state IN ('claimed', 'delivered', 'definitely_unsent', 'uncertain')),
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 1 AND 9007199254740991)
);
CREATE UNIQUE INDEX focus_one_active_checklist ON focus_checklists(identity_id) WHERE state = 'claimed';
CREATE TABLE focus_items (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT CHECK (sequence BETWEEN 1 AND 9007199254740991),
    identity_id TEXT NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
    request_id TEXT NOT NULL REFERENCES request_attempts(request_id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('decision', 'review', 'fyi', 'result')),
    source TEXT NOT NULL CHECK (source IN ('incoming', 'result', 'timeout')),
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms BETWEEN 1 AND 9007199254740991),
    checklist_id TEXT REFERENCES focus_checklists(id),
    UNIQUE(identity_id, request_id, source)
);
CREATE INDEX focus_items_pending ON focus_items(identity_id, checklist_id, sequence);

CREATE TRIGGER focus_policies_advances_change_cursor_on_insert AFTER INSERT ON focus_policies
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;

CREATE TRIGGER focus_policies_advances_change_cursor_on_update AFTER UPDATE ON focus_policies
WHEN OLD.identity_id IS NOT NEW.identity_id
  OR OLD.revision IS NOT NEW.revision
  OR OLD.until_ms IS NOT NEW.until_ms
  OR OLD.owner_identity_id IS NOT NEW.owner_identity_id
  OR OLD.setter_identity_id IS NOT NEW.setter_identity_id
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;

CREATE TRIGGER focus_policies_advances_change_cursor_on_delete AFTER DELETE ON focus_policies
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;

CREATE TRIGGER request_delivery_policies_advances_change_cursor_on_insert AFTER INSERT ON request_delivery_policies
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;

CREATE TRIGGER request_delivery_policies_advances_change_cursor_on_update AFTER UPDATE ON request_delivery_policies
WHEN OLD.request_id IS NOT NEW.request_id
  OR OLD.urgent IS NOT NEW.urgent
  OR OLD.kind IS NOT NEW.kind
  OR OLD.automatic IS NOT NEW.automatic
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;

CREATE TRIGGER request_delivery_policies_advances_change_cursor_on_delete AFTER DELETE ON request_delivery_policies
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;

CREATE TRIGGER focus_checklists_advances_change_cursor_on_insert AFTER INSERT ON focus_checklists
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;

CREATE TRIGGER focus_checklists_advances_change_cursor_on_update AFTER UPDATE ON focus_checklists
WHEN OLD.id IS NOT NEW.id
  OR OLD.identity_id IS NOT NEW.identity_id
  OR OLD.attempt_token IS NOT NEW.attempt_token
  OR OLD.through_sequence IS NOT NEW.through_sequence
  OR OLD.state IS NOT NEW.state
  OR OLD.created_at_ms IS NOT NEW.created_at_ms
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;

CREATE TRIGGER focus_checklists_advances_change_cursor_on_delete AFTER DELETE ON focus_checklists
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;

CREATE TRIGGER focus_items_advances_change_cursor_on_insert AFTER INSERT ON focus_items
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;

CREATE TRIGGER focus_items_advances_change_cursor_on_update AFTER UPDATE ON focus_items
WHEN OLD.sequence IS NOT NEW.sequence
  OR OLD.identity_id IS NOT NEW.identity_id
  OR OLD.request_id IS NOT NEW.request_id
  OR OLD.kind IS NOT NEW.kind
  OR OLD.source IS NOT NEW.source
  OR OLD.created_at_ms IS NOT NEW.created_at_ms
  OR OLD.checklist_id IS NOT NEW.checklist_id
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;

CREATE TRIGGER focus_items_advances_change_cursor_on_delete AFTER DELETE ON focus_items
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
