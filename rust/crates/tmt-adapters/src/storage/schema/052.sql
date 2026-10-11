-- Empty participant IDs represent unknown provenance, never a guessed identity.
-- No foreign keys: identity retirement and request/body expiry preserve these totals.
CREATE TABLE exchange_flow (
 sender_identity_id TEXT NOT NULL,
 recipient_identity_id TEXT NOT NULL,
 kind TEXT NOT NULL CHECK (kind IN ('talk','reply','result','ack')),
 bucket_start_ms INTEGER NOT NULL CHECK (typeof(bucket_start_ms) = 'integer' AND bucket_start_ms BETWEEN 0 AND 9007199254740991 AND bucket_start_ms % 1800000 = 0),
 count INTEGER NOT NULL CHECK (typeof(count) = 'integer' AND count BETWEEN 1 AND 9007199254740991),
 bytes INTEGER NOT NULL CHECK (typeof(bytes) = 'integer' AND bytes BETWEEN 0 AND 9007199254740991 AND (kind IN ('talk','reply') OR bytes = 0)),
 PRIMARY KEY(sender_identity_id,recipient_identity_id,kind,bucket_start_ms)
);
CREATE INDEX exchange_flow_bucket ON exchange_flow(bucket_start_ms);
CREATE TRIGGER exchange_flow_advances_change_cursor_on_insert AFTER INSERT ON exchange_flow
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER exchange_flow_advances_change_cursor_on_update AFTER UPDATE ON exchange_flow
WHEN OLD.sender_identity_id IS NOT NEW.sender_identity_id
 OR OLD.recipient_identity_id IS NOT NEW.recipient_identity_id
 OR OLD.kind IS NOT NEW.kind OR OLD.bucket_start_ms IS NOT NEW.bucket_start_ms
 OR OLD.count IS NOT NEW.count OR OLD.bytes IS NOT NEW.bytes
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
CREATE TRIGGER exchange_flow_advances_change_cursor_on_delete AFTER DELETE ON exchange_flow
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
