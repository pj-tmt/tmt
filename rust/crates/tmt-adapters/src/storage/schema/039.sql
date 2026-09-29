-- A second terminal host (Herdr, #479). SQLite cannot alter a CHECK, so
-- bindings is rebuilt with only its transport CHECK widened: every other
-- column, rule and uniqueness constraint is copied verbatim, in order. The
-- migration first refuses a bindings table that differs from schema 38.
CREATE TABLE bindings_039 (
  id TEXT PRIMARY KEY,
  identity_id TEXT NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
  transport TEXT NOT NULL CHECK (transport IN ('tmux', 'herdr')),
  pane_id TEXT NOT NULL,
  server_id TEXT NOT NULL,
  socket_path TEXT NOT NULL,
  server_pid INTEGER NOT NULL,
  server_start_time TEXT NOT NULL,
  pane_pid INTEGER NOT NULL,
  bound_at TEXT NOT NULL,
  last_verified_at TEXT NOT NULL, runtime_state TEXT NOT NULL DEFAULT 'unknown'
  CHECK (runtime_state IN ('unknown', 'running', 'ended')), last_transition TEXT
  CHECK (last_transition IN ('started', 'resumed', 'cleared', 'compacted', 'forked', 'ended')), runtime_pid INTEGER CHECK (runtime_pid > 0 AND runtime_pid <= 9007199254740991), runtime_start_identity TEXT, observed_provider_session_id TEXT
  CHECK ((runtime_pid IS NULL AND runtime_start_identity IS NULL AND observed_provider_session_id IS NULL)
    OR (runtime_pid IS NOT NULL AND runtime_start_identity IS NOT NULL))
  CHECK (runtime_state = 'unknown' OR runtime_pid IS NOT NULL), launch_owner_pid INTEGER
  CHECK (launch_owner_pid > 0 AND launch_owner_pid <= 9007199254740991), launch_owner_start_identity TEXT
  CHECK ((launch_owner_pid IS NULL AND launch_owner_start_identity IS NULL)
    OR (launch_owner_pid IS NOT NULL AND launch_owner_start_identity IS NOT NULL AND runtime_pid IS NOT NULL)),
  UNIQUE(identity_id),
  UNIQUE(transport, server_id, pane_id)
);
INSERT INTO bindings_039 (
  id, identity_id, transport, pane_id, server_id, socket_path, server_pid,
  server_start_time, pane_pid, bound_at, last_verified_at, runtime_state,
  last_transition, runtime_pid, runtime_start_identity,
  observed_provider_session_id, launch_owner_pid, launch_owner_start_identity
)
SELECT
  id, identity_id, transport, pane_id, server_id, socket_path, server_pid,
  server_start_time, pane_pid, bound_at, last_verified_at, runtime_state,
  last_transition, runtime_pid, runtime_start_identity,
  observed_provider_session_id, launch_owner_pid, launch_owner_start_identity
FROM bindings;
DROP TABLE bindings;
ALTER TABLE bindings_039 RENAME TO bindings;
CREATE INDEX bindings_endpoint ON bindings(transport, server_id, pane_id);

-- The request fence records its host. NULL is tmux, the only host that wrote
-- earlier rows, so history is never rewritten; inbox routes have no endpoint.
ALTER TABLE request_attempts ADD COLUMN host TEXT
  CHECK (host IS NULL OR (route_kind = 'pane' AND host IN ('tmux', 'herdr')));
ALTER TABLE request_responses ADD COLUMN host TEXT
  CHECK (host IS NULL OR host IN ('tmux', 'herdr'));

-- A TMT-generated UUIDv4 for each server incarnation of a host that has no
-- server-level store of its own (Herdr). tmux keeps its ID in a server option.
CREATE TABLE host_servers (
  server_id TEXT PRIMARY KEY CHECK (length(server_id) = 36),
  host TEXT NOT NULL CHECK (host IN ('herdr')),
  socket_path TEXT NOT NULL CHECK (length(socket_path) > 0),
  server_pid INTEGER NOT NULL
    CHECK (typeof(server_pid) = 'integer' AND server_pid BETWEEN 1 AND 9007199254740991),
  server_start_time TEXT NOT NULL CHECK (length(server_start_time) > 0),
  first_seen_at_ms INTEGER NOT NULL
    CHECK (typeof(first_seen_at_ms) = 'integer' AND first_seen_at_ms BETWEEN 0 AND 9007199254740991),
  UNIQUE(host, socket_path, server_pid, server_start_time)
);
