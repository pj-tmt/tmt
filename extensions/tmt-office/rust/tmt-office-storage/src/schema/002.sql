-- Written once, after core records the Office storage cutover receipt, when
-- this database becomes authoritative. Its absence under a receipt means the
-- switch committed but activation did not finish.
CREATE TABLE _office_activation (
  singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
  manifest TEXT NOT NULL CHECK (length(manifest) = 64),
  switched_at_ms INTEGER NOT NULL CHECK (typeof(switched_at_ms) = 'integer' AND switched_at_ms > 0),
  activated_at_ms INTEGER NOT NULL CHECK (typeof(activated_at_ms) = 'integer' AND activated_at_ms > 0)
);
