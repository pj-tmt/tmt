-- Durable storage cutover receipts for extensions that move their data out of
-- the shared database. Committing a receipt is the extension's single decision
-- point; the extension's migration coordinator writes it in the same
-- transaction that rechecks its source rows.
CREATE TABLE extension_storage_cutovers (
  extension TEXT PRIMARY KEY
    CHECK (length(extension) BETWEEN 1 AND 32 AND extension NOT GLOB '*[^a-z0-9-]*'),
  destination_device INTEGER NOT NULL CHECK (typeof(destination_device) = 'integer'),
  destination_inode INTEGER NOT NULL CHECK (typeof(destination_inode) = 'integer'),
  storage_schema_version INTEGER NOT NULL CHECK (typeof(storage_schema_version) = 'integer' AND storage_schema_version >= 1),
  manifest TEXT NOT NULL CHECK (length(manifest) = 64),
  switched_at_ms INTEGER NOT NULL CHECK (typeof(switched_at_ms) = 'integer' AND switched_at_ms > 0)
) WITHOUT ROWID;

-- After the Office receipt, the retained Office rows are read-only. These
-- triggers also fence writers whose connections were opened by older binaries.
-- A later core cleanup removes them together with the Office tables.
CREATE TRIGGER office_local_blocks_after_office_cutover_insert BEFORE INSERT ON office_local_blocks
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_local_blocks_after_office_cutover_update BEFORE UPDATE ON office_local_blocks
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_local_blocks_after_office_cutover_delete BEFORE DELETE ON office_local_blocks
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_local_worlds_after_office_cutover_insert BEFORE INSERT ON office_local_worlds
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_local_worlds_after_office_cutover_update BEFORE UPDATE ON office_local_worlds
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_local_worlds_after_office_cutover_delete BEFORE DELETE ON office_local_worlds
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_local_profiles_after_office_cutover_insert BEFORE INSERT ON office_local_profiles
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_local_profiles_after_office_cutover_update BEFORE UPDATE ON office_local_profiles
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_local_profiles_after_office_cutover_delete BEFORE DELETE ON office_local_profiles
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_board_state_after_office_cutover_insert BEFORE INSERT ON office_board_state
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_board_state_after_office_cutover_update BEFORE UPDATE ON office_board_state
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_board_state_after_office_cutover_delete BEFORE DELETE ON office_board_state
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_board_entries_after_office_cutover_insert BEFORE INSERT ON office_board_entries
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_board_entries_after_office_cutover_update BEFORE UPDATE ON office_board_entries
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_board_entries_after_office_cutover_delete BEFORE DELETE ON office_board_entries
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_board_operations_after_office_cutover_insert BEFORE INSERT ON office_board_operations
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_board_operations_after_office_cutover_update BEFORE UPDATE ON office_board_operations
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_board_operations_after_office_cutover_delete BEFORE DELETE ON office_board_operations
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_whiteboards_after_office_cutover_insert BEFORE INSERT ON office_whiteboards
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_whiteboards_after_office_cutover_update BEFORE UPDATE ON office_whiteboards
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_whiteboards_after_office_cutover_delete BEFORE DELETE ON office_whiteboards
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_whiteboard_operations_after_office_cutover_insert BEFORE INSERT ON office_whiteboard_operations
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_whiteboard_operations_after_office_cutover_update BEFORE UPDATE ON office_whiteboard_operations
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_whiteboard_operations_after_office_cutover_delete BEFORE DELETE ON office_whiteboard_operations
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_whiteboard_snapshots_after_office_cutover_insert BEFORE INSERT ON office_whiteboard_snapshots
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_whiteboard_snapshots_after_office_cutover_update BEFORE UPDATE ON office_whiteboard_snapshots
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_whiteboard_snapshots_after_office_cutover_delete BEFORE DELETE ON office_whiteboard_snapshots
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_whiteboard_snapshot_images_after_office_cutover_insert BEFORE INSERT ON office_whiteboard_snapshot_images
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_whiteboard_snapshot_images_after_office_cutover_update BEFORE UPDATE ON office_whiteboard_snapshot_images
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_whiteboard_snapshot_images_after_office_cutover_delete BEFORE DELETE ON office_whiteboard_snapshot_images
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_prop_packs_after_office_cutover_insert BEFORE INSERT ON office_prop_packs
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_prop_packs_after_office_cutover_update BEFORE UPDATE ON office_prop_packs
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_prop_packs_after_office_cutover_delete BEFORE DELETE ON office_prop_packs
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_prop_catalog_after_office_cutover_insert BEFORE INSERT ON office_prop_catalog
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_prop_catalog_after_office_cutover_update BEFORE UPDATE ON office_prop_catalog
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_prop_catalog_after_office_cutover_delete BEFORE DELETE ON office_prop_catalog
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_avatar_packs_after_office_cutover_insert BEFORE INSERT ON office_avatar_packs
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_avatar_packs_after_office_cutover_update BEFORE UPDATE ON office_avatar_packs
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_avatar_packs_after_office_cutover_delete BEFORE DELETE ON office_avatar_packs
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_avatar_catalog_after_office_cutover_insert BEFORE INSERT ON office_avatar_catalog
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_avatar_catalog_after_office_cutover_update BEFORE UPDATE ON office_avatar_catalog
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
CREATE TRIGGER office_avatar_catalog_after_office_cutover_delete BEFORE DELETE ON office_avatar_catalog
WHEN EXISTS (SELECT 1 FROM extension_storage_cutovers WHERE extension = 'office')
BEGIN SELECT RAISE(ABORT, 'Office data moved to Office storage'); END;
