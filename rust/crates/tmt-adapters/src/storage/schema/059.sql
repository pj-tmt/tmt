-- How a request is delivered: a request carries its reply command, a passthrough is a
-- person's own text typed into the agent's pane as is. It lives with the other per-request
-- delivery facts, so the request table and its change-cursor trigger stay as they are.
ALTER TABLE request_delivery_policies
    ADD COLUMN mode TEXT NOT NULL DEFAULT 'request' CHECK (mode IN ('request', 'passthrough'));

DROP TRIGGER request_delivery_policies_advances_change_cursor_on_update;
CREATE TRIGGER request_delivery_policies_advances_change_cursor_on_update AFTER UPDATE ON request_delivery_policies
WHEN OLD.request_id IS NOT NEW.request_id
  OR OLD.urgent IS NOT NEW.urgent
  OR OLD.kind IS NOT NEW.kind
  OR OLD.automatic IS NOT NEW.automatic
  OR OLD.mode IS NOT NEW.mode
BEGIN UPDATE change_cursor SET value = value + 1 WHERE id = 1; END;
