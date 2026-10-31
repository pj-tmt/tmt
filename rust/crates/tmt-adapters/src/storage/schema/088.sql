-- Sending rules and talk limits (#4892). Leads are an explicit list; a retired identity
-- stays stored and drops out at read time.
CREATE TABLE orchestration_leads (
 identity_id TEXT NOT NULL PRIMARY KEY REFERENCES identities(id) ON DELETE CASCADE
);
CREATE TRIGGER orchestration_leads_advances_change_cursor_on_insert AFTER INSERT ON orchestration_leads
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER orchestration_leads_advances_change_cursor_on_update AFTER UPDATE ON orchestration_leads
WHEN OLD.identity_id IS NOT NEW.identity_id
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER orchestration_leads_advances_change_cursor_on_delete AFTER DELETE ON orchestration_leads
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;

-- `route allow`: one ordered pair passes its sending rule until until_ms.
CREATE TABLE orchestration_route_grants (
 from_id TEXT NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
 to_id TEXT NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
 until_ms INTEGER NOT NULL CHECK (typeof(until_ms) = 'integer' AND until_ms BETWEEN 1 AND 9007199254740991),
 granted_by TEXT,
 PRIMARY KEY(from_id,to_id),
 CHECK (from_id <> to_id)
);
CREATE TRIGGER orchestration_route_grants_advances_change_cursor_on_insert AFTER INSERT ON orchestration_route_grants
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER orchestration_route_grants_advances_change_cursor_on_update AFTER UPDATE ON orchestration_route_grants
WHEN OLD.from_id IS NOT NEW.from_id OR OLD.to_id IS NOT NEW.to_id OR OLD.until_ms IS NOT NEW.until_ms OR OLD.granted_by IS NOT NEW.granted_by
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER orchestration_route_grants_advances_change_cursor_on_delete AFTER DELETE ON orchestration_route_grants
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;

-- `talks resume`: the pair window restarts here. Members are in canonical UUID order.
CREATE TABLE orchestration_talk_resumes (
 first_id TEXT NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
 second_id TEXT NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
 resumed_ms INTEGER NOT NULL CHECK (typeof(resumed_ms) = 'integer' AND resumed_ms BETWEEN 1 AND 9007199254740991),
 PRIMARY KEY(first_id,second_id),
 CHECK (first_id < second_id)
);
CREATE TRIGGER orchestration_talk_resumes_advances_change_cursor_on_insert AFTER INSERT ON orchestration_talk_resumes
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER orchestration_talk_resumes_advances_change_cursor_on_update AFTER UPDATE ON orchestration_talk_resumes
WHEN OLD.first_id IS NOT NEW.first_id OR OLD.second_id IS NOT NEW.second_id OR OLD.resumed_ms IS NOT NEW.resumed_ms
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER orchestration_talk_resumes_advances_change_cursor_on_delete AFTER DELETE ON orchestration_talk_resumes
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;

-- A request held at its pair's talk limit ("needs a person"). Its attempt stays
-- not_attempted with no wake and no Digest item; resume deletes the row and wakes it
-- through normal delivery.
CREATE TABLE orchestration_talk_holds (
 request_id TEXT NOT NULL PRIMARY KEY REFERENCES request_attempts(request_id) ON DELETE CASCADE,
 from_id TEXT NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
 to_id TEXT NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
 held_ms INTEGER NOT NULL CHECK (typeof(held_ms) = 'integer' AND held_ms BETWEEN 1 AND 9007199254740991)
);
CREATE INDEX orchestration_talk_holds_by_pair ON orchestration_talk_holds(from_id,to_id);
CREATE TRIGGER orchestration_talk_holds_advances_change_cursor_on_insert AFTER INSERT ON orchestration_talk_holds
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER orchestration_talk_holds_advances_change_cursor_on_update AFTER UPDATE ON orchestration_talk_holds
WHEN OLD.request_id IS NOT NEW.request_id OR OLD.from_id IS NOT NEW.from_id OR OLD.to_id IS NOT NEW.to_id OR OLD.held_ms IS NOT NEW.held_ms
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER orchestration_talk_holds_advances_change_cursor_on_delete AFTER DELETE ON orchestration_talk_holds
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
