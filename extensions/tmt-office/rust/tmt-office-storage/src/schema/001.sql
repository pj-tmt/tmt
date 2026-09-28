-- Office storage schema v1: the Office-owned tables exactly as core schema 35
-- defines them, so migration copies raw cells without reinterpretation. The
-- only difference is the two identities(id) references: identities stay
-- core-owned, and Office preflights them through CoreAccess instead.
CREATE TABLE office_avatar_catalog (
  singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
  revision INTEGER NOT NULL CHECK (revision >= 0 AND revision <= 9007199254740991),
  previous_kind TEXT CHECK (previous_kind IN ('install', 'remove')),
  previous_digest TEXT,
  previous_base_revision INTEGER CHECK (previous_base_revision >= 0 AND previous_base_revision <= 9007199254740991),
  previous_result_revision INTEGER CHECK (previous_result_revision >= 1 AND previous_result_revision <= 9007199254740991),
  CHECK (
    (previous_kind IS NULL AND previous_digest IS NULL AND previous_base_revision IS NULL AND previous_result_revision IS NULL)
    OR
    (previous_kind IS NOT NULL AND previous_digest IS NOT NULL AND previous_base_revision IS NOT NULL AND previous_result_revision = previous_base_revision + 1)
  )
);
CREATE TABLE office_avatar_packs (
  digest TEXT PRIMARY KEY COLLATE BINARY
    CHECK (length(digest) = 71 AND substr(digest, 1, 7) = 'sha256:'),
  bytes BLOB NOT NULL CHECK (length(bytes) <= 32768),
  avatar_count INTEGER NOT NULL CHECK (avatar_count BETWEEN 1 AND 16),
  installed_revision INTEGER NOT NULL UNIQUE
    CHECK (installed_revision >= 1 AND installed_revision <= 9007199254740991),
  installed_at_ms INTEGER NOT NULL CHECK (installed_at_ms >= 0)
);
CREATE TABLE "office_board_entries" (
  id TEXT PRIMARY KEY CHECK (length(id) = 36),
  thread_id TEXT NOT NULL CHECK (length(thread_id) = 36),
  is_root INTEGER NOT NULL CHECK (is_root IN (0, 1)),
  category_kind TEXT NOT NULL CHECK (category_kind IN ('general', 'repository', 'room')),
  category_id TEXT,
  author_kind TEXT NOT NULL CHECK (author_kind IN ('owner', 'identity')),
  author_id TEXT NOT NULL,
  author_name TEXT,
  revision INTEGER NOT NULL CHECK (revision BETWEEN 1 AND 9007199254740991),
  deleted INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
  created_sequence INTEGER NOT NULL UNIQUE,
  activity_sequence INTEGER NOT NULL,
  created_at_ms INTEGER NOT NULL,
  updated_at_ms INTEGER NOT NULL,
  title TEXT,
  body TEXT,
  CHECK ((category_kind = 'general' AND category_id IS NULL) OR
         (category_kind = 'repository' AND category_id IS NOT NULL) OR
         (category_kind = 'room' AND category_id IS NOT NULL AND length(category_id) = 36)),
  CHECK ((is_root = 1 AND thread_id = id) OR is_root = 0),
  CHECK ((author_kind = 'owner' AND author_name IS NULL) OR (author_kind = 'identity' AND author_name IS NOT NULL))
);
CREATE TABLE office_board_operations (
  actor_key TEXT NOT NULL,
  operation_id TEXT NOT NULL CHECK (length(operation_id) = 36),
  intent_digest TEXT NOT NULL CHECK (length(intent_digest) = 64),
  result_kind TEXT NOT NULL CHECK (result_kind IN ('create', 'edit', 'delete')),
  entry_id TEXT NOT NULL CHECK (length(entry_id) = 36),
  thread_id TEXT,
  revision INTEGER NOT NULL,
  created INTEGER,
  changed INTEGER,
  deleted INTEGER,
  moderated INTEGER,
  PRIMARY KEY (actor_key, operation_id)
) WITHOUT ROWID;
CREATE TABLE office_board_state (
  singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
  revision INTEGER NOT NULL CHECK (revision BETWEEN 0 AND 9007199254740991),
  next_sequence INTEGER NOT NULL CHECK (next_sequence BETWEEN 1 AND 9007199254740991)
);
CREATE TABLE office_local_blocks (
  block_id TEXT PRIMARY KEY CHECK (length(block_id) = 36),
  target_kind TEXT NOT NULL DEFAULT 'identity' CHECK (target_kind IN ('identity', 'lobby')),
  identity_id TEXT UNIQUE,
  revision INTEGER NOT NULL CHECK (
    typeof(revision) = 'integer' AND revision BETWEEN 1 AND 9007199254740991
  ),
  layout TEXT NOT NULL CHECK (length(CAST(layout AS BLOB)) <= 8192),
  updated_at_ms INTEGER NOT NULL CHECK (
    typeof(updated_at_ms) = 'integer' AND
    updated_at_ms > 0 AND updated_at_ms <= 9007199254740991
  ),
  CHECK ((target_kind = 'identity' AND identity_id IS NOT NULL) OR
         (target_kind = 'lobby' AND identity_id IS NULL))
);
CREATE TABLE office_local_profiles (
  identity_id TEXT PRIMARY KEY,
  revision INTEGER NOT NULL CHECK (
    typeof(revision) = 'integer' AND
    revision BETWEEN 1 AND 9007199254740991
  ),
  profile TEXT NOT NULL CHECK (length(CAST(profile AS BLOB)) <= 4096),
  updated_at_ms INTEGER NOT NULL CHECK (
    typeof(updated_at_ms) = 'integer' AND
    updated_at_ms > 0 AND updated_at_ms <= 9007199254740991
  )
);
CREATE TABLE office_local_worlds (
  singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
  id TEXT NOT NULL UNIQUE CHECK (length(id) = 36),
  created_at_ms INTEGER NOT NULL CHECK (
    typeof(created_at_ms) = 'integer' AND
    created_at_ms > 0 AND created_at_ms <= 9007199254740991
  )
, layout_revision INTEGER NOT NULL DEFAULT 0
  CHECK (typeof(layout_revision) = 'integer' AND layout_revision BETWEEN 0 AND 9007199254740991), layout_json TEXT
  CHECK (layout_json IS NULL OR length(CAST(layout_json AS BLOB)) <= 4194304), layout_updated_at_ms INTEGER NOT NULL DEFAULT 0
  CHECK (typeof(layout_updated_at_ms) = 'integer' AND layout_updated_at_ms BETWEEN 0 AND 9007199254740991));
CREATE TABLE office_prop_catalog (
  singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
  revision INTEGER NOT NULL CHECK (revision >= 0 AND revision <= 9007199254740991),
  previous_kind TEXT CHECK (previous_kind IN ('install', 'remove')),
  previous_digest TEXT,
  previous_base_revision INTEGER CHECK (previous_base_revision >= 0 AND previous_base_revision <= 9007199254740991),
  previous_result_revision INTEGER CHECK (previous_result_revision >= 1 AND previous_result_revision <= 9007199254740991),
  CHECK (
    (previous_kind IS NULL AND previous_digest IS NULL AND previous_base_revision IS NULL AND previous_result_revision IS NULL)
    OR
    (previous_kind IS NOT NULL AND previous_digest IS NOT NULL AND previous_base_revision IS NOT NULL AND previous_result_revision = previous_base_revision + 1)
  )
);
CREATE TABLE office_prop_packs (
  digest TEXT PRIMARY KEY COLLATE BINARY
    CHECK (length(digest) = 71 AND substr(digest, 1, 7) = 'sha256:'),
  bytes BLOB NOT NULL CHECK (length(bytes) <= 524288),
  prop_count INTEGER NOT NULL CHECK (prop_count BETWEEN 1 AND 16),
  installed_revision INTEGER NOT NULL UNIQUE
    CHECK (installed_revision >= 1 AND installed_revision <= 9007199254740991),
  installed_at_ms INTEGER NOT NULL CHECK (installed_at_ms >= 0)
);
CREATE TABLE office_whiteboard_operations (
  world_id TEXT NOT NULL REFERENCES office_local_worlds(id),
  operation_id TEXT NOT NULL CHECK (length(operation_id) = 36),
  intent_digest TEXT NOT NULL CHECK (length(intent_digest) = 64),
  document_id TEXT NOT NULL REFERENCES office_whiteboards(document_id),
  revision INTEGER NOT NULL CHECK (typeof(revision) = 'integer' AND revision BETWEEN 1 AND 9007199254740991),
  changed INTEGER NOT NULL CHECK (changed IN (0, 1)),
  updated_at_ms INTEGER NOT NULL CHECK (typeof(updated_at_ms) = 'integer' AND updated_at_ms BETWEEN 1 AND 9007199254740991),
  PRIMARY KEY (world_id, operation_id)
) WITHOUT ROWID;
CREATE TABLE office_whiteboard_snapshot_images (
  snapshot_id TEXT PRIMARY KEY NOT NULL REFERENCES office_whiteboard_snapshots(snapshot_id),
  pixel_digest TEXT NOT NULL CHECK (length(pixel_digest) = 64),
  png BLOB NOT NULL CHECK (typeof(png) = 'blob' AND length(png) BETWEEN 1 AND 8388608)
);
CREATE TABLE office_whiteboard_snapshots (
  snapshot_id TEXT PRIMARY KEY NOT NULL CHECK (length(snapshot_id) = 36),
  world_id TEXT NOT NULL REFERENCES office_local_worlds(id),
  intent_digest TEXT NOT NULL CHECK (length(intent_digest) = 64),
  document_id TEXT NOT NULL CHECK (document_id = 'lobby' OR length(document_id) = 36),
  document_revision INTEGER NOT NULL CHECK (typeof(document_revision) = 'integer' AND document_revision BETWEEN 1 AND 9007199254740991),
  scene TEXT NOT NULL CHECK (length(CAST(scene AS BLOB)) <= 2097152),
  selected_element_ids TEXT NOT NULL CHECK (length(CAST(selected_element_ids AS BLOB)) <= 81920),
  annotation TEXT NOT NULL CHECK (length(CAST(annotation AS BLOB)) <= 16384),
  created_at_ms INTEGER NOT NULL CHECK (typeof(created_at_ms) = 'integer' AND created_at_ms BETWEEN 1 AND 9007199254740991)
);
CREATE TABLE office_whiteboards (
  document_id TEXT PRIMARY KEY CHECK (document_id = 'lobby' OR length(document_id) = 36),
  world_id TEXT NOT NULL REFERENCES office_local_worlds(id),
  revision INTEGER NOT NULL CHECK (typeof(revision) = 'integer' AND revision BETWEEN 1 AND 9007199254740991),
  scene TEXT NOT NULL CHECK (length(CAST(scene AS BLOB)) <= 2097152),
  updated_at_ms INTEGER NOT NULL CHECK (typeof(updated_at_ms) = 'integer' AND updated_at_ms BETWEEN 1 AND 9007199254740991)
);
CREATE INDEX office_board_author ON office_board_entries
  (category_kind, category_id, is_root, author_kind, author_id, created_sequence DESC);
CREATE INDEX office_board_categories ON office_board_entries
  (is_root, category_kind, category_id);
CREATE INDEX office_board_recent ON office_board_entries
  (category_kind, category_id, is_root, created_sequence DESC, id DESC);
CREATE INDEX office_board_replies ON office_board_entries
  (thread_id, is_root, created_sequence, id);
CREATE INDEX office_board_updated ON office_board_entries
  (category_kind, category_id, is_root, activity_sequence DESC, id DESC);
CREATE INDEX office_local_blocks_active_projection ON office_local_blocks (identity_id, block_id);
CREATE UNIQUE INDEX office_local_blocks_single_lobby
  ON office_local_blocks (target_kind) WHERE target_kind = 'lobby';
CREATE TRIGGER office_blocks_after_world_insert BEFORE INSERT ON office_local_blocks
WHEN EXISTS (SELECT 1 FROM office_local_worlds WHERE layout_revision > 0)
BEGIN SELECT RAISE(ABORT, 'Office layout uses the world editor'); END;
CREATE TRIGGER office_blocks_after_world_update BEFORE UPDATE ON office_local_blocks
WHEN EXISTS (SELECT 1 FROM office_local_worlds WHERE layout_revision > 0)
BEGIN SELECT RAISE(ABORT, 'Office layout uses the world editor'); END;

-- Office-only state. Nothing below exists in the core database.
CREATE TABLE _office_schema (
  version INTEGER PRIMARY KEY CHECK (typeof(version) = 'integer' AND version >= 1),
  name TEXT NOT NULL
);
-- Identities whose retirement the Office consumer has processed; writes whose
-- retirement consequence is an action (pairing grants) check it transactionally.
CREATE TABLE office_retired_identities (
  identity_id TEXT PRIMARY KEY CHECK (length(identity_id) = 36),
  retired_at_ms INTEGER NOT NULL CHECK (typeof(retired_at_ms) = 'integer' AND retired_at_ms >= 0),
  recorded_at_ms INTEGER NOT NULL CHECK (typeof(recorded_at_ms) = 'integer' AND recorded_at_ms >= 0)
) WITHOUT ROWID;
-- One resumable migration record; it stays with a migrated database as provenance.
CREATE TABLE _office_migration (
  singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
  state TEXT NOT NULL CHECK (state IN ('prepared', 'copied', 'verified')),
  source_path TEXT NOT NULL,
  source_device INTEGER NOT NULL,
  source_inode INTEGER NOT NULL,
  source_schema_version INTEGER NOT NULL,
  source_manifest TEXT CHECK (source_manifest IS NULL OR length(source_manifest) = 64),
  prepared_at_ms INTEGER NOT NULL,
  copied_at_ms INTEGER,
  verified_at_ms INTEGER,
  CHECK ((state = 'prepared' AND source_manifest IS NULL AND copied_at_ms IS NULL AND verified_at_ms IS NULL)
      OR (state = 'copied' AND source_manifest IS NOT NULL AND copied_at_ms IS NOT NULL AND verified_at_ms IS NULL)
      OR (state = 'verified' AND source_manifest IS NOT NULL AND copied_at_ms IS NOT NULL AND verified_at_ms IS NOT NULL))
);
