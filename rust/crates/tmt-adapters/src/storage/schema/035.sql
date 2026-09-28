-- Hints are independently claimed effects, not another request or final store.
-- Old requests remain silent; only newly opted-in requests acquire a row.
CREATE TABLE request_notifications (
    request_id TEXT PRIMARY KEY REFERENCES request_attempts(request_id) ON DELETE CASCADE,
    deadline_ms INTEGER NOT NULL CHECK (deadline_ms > 0 AND deadline_ms <= 9007199254740991),
    timeout_ms INTEGER NOT NULL CHECK (timeout_ms > 0 AND timeout_ms <= 86400000),
    waiter_pid INTEGER CHECK (waiter_pid > 0 AND waiter_pid <= 9007199254740991),
    waiter_start TEXT,
    reply_state TEXT NOT NULL DEFAULT 'not_attempted'
      CHECK (reply_state IN ('not_attempted', 'claimed', 'sent', 'unavailable', 'uncertain')),
    timeout_state TEXT NOT NULL DEFAULT 'not_attempted'
      CHECK (timeout_state IN ('not_attempted', 'claimed', 'sent', 'unavailable', 'uncertain')),
    observed INTEGER NOT NULL DEFAULT 0 CHECK (observed IN (0, 1)),
    CHECK ((waiter_pid IS NULL) = (waiter_start IS NULL))
);
