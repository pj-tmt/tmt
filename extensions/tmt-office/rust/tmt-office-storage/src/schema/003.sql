-- Rooms Office confirmed retired in core. Monotonic: reconciliation only
-- inserts, and Office transactions that write a room reference check it.
CREATE TABLE office_retired_rooms (
  room_id TEXT PRIMARY KEY CHECK (length(room_id) = 36),
  recorded_at_ms INTEGER NOT NULL CHECK (typeof(recorded_at_ms) = 'integer' AND recorded_at_ms >= 0)
) WITHOUT ROWID;
