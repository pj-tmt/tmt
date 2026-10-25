-- Frozen Remote predecessor schema, copied from 4dcc50f03203e5dfcfa5f2ca6e3016f990f29354.
-- Includes the original 13 migration definitions/history; no M1 implementation generates expected rows.
PRAGMA foreign_keys=ON;
CREATE TABLE _migrations(version INTEGER PRIMARY KEY,name TEXT NOT NULL,applied_at TEXT NOT NULL);
CREATE TABLE machine(
             singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
             machine_id TEXT NOT NULL,
             route_prefix TEXT NOT NULL);
INSERT INTO _migrations VALUES(1,'machine','2026-10-24');
CREATE TABLE grants(
             client_id TEXT PRIMARY KEY,
             public_key BLOB NOT NULL CHECK(length(public_key) = 32),
             kind TEXT NOT NULL CHECK(kind IN ('addon', 'browser', 'cli')),
             origin TEXT NOT NULL,
             name TEXT NOT NULL,
             agents TEXT NOT NULL,
             scopes TEXT NOT NULL,
             mode TEXT NOT NULL CHECK(mode IN ('direct', 'hold')),
             issued_at_ms INTEGER NOT NULL,
             expires_at_ms INTEGER,
             revision INTEGER NOT NULL CHECK(revision > 0),
             disabled INTEGER NOT NULL CHECK(disabled IN (0, 1)));
         -- One live grant per device key; a revoked key may pair again.
         CREATE UNIQUE INDEX grants_live_key ON grants(public_key) WHERE disabled = 0;
INSERT INTO _migrations VALUES(2,'grants','2026-10-24');
CREATE TABLE sessions(
             client_id TEXT PRIMARY KEY REFERENCES grants(client_id),
             session_id TEXT NOT NULL UNIQUE,
             window_id TEXT NOT NULL,
             grant_revision INTEGER NOT NULL,
             next_client_sequence TEXT NOT NULL,
             next_server_sequence TEXT NOT NULL);
INSERT INTO _migrations VALUES(3,'sessions','2026-10-24');
CREATE TABLE streams(client_id TEXT PRIMARY KEY REFERENCES grants(client_id),
            incarnation TEXT NOT NULL, key BLOB NOT NULL, tip INTEGER NOT NULL DEFAULT 0,
            floor INTEGER NOT NULL DEFAULT 0, observed INTEGER NOT NULL DEFAULT 0,
            acknowledged INTEGER NOT NULL DEFAULT 0,last_ms INTEGER NOT NULL DEFAULT 0);
         CREATE TABLE entries(client_id TEXT NOT NULL REFERENCES streams(client_id),
            position INTEGER NOT NULL, envelope BLOB NOT NULL, at_ms INTEGER NOT NULL,
            PRIMARY KEY(client_id,position));
         CREATE TABLE operations(id TEXT PRIMARY KEY, client_id TEXT NOT NULL REFERENCES grants(client_id),
            operation TEXT NOT NULL, digest BLOB NOT NULL, frozen BLOB, phase TEXT NOT NULL,
            receipt TEXT NOT NULL, references_json TEXT NOT NULL DEFAULT '[]', updated_ms INTEGER NOT NULL);
         CREATE INDEX operations_client ON operations(client_id);
         CREATE TABLE budgets(subject TEXT NOT NULL,kind TEXT NOT NULL,started_ms INTEGER NOT NULL,
            used INTEGER NOT NULL,PRIMARY KEY(subject,kind));
         CREATE TABLE audit(position INTEGER PRIMARY KEY,at_ms INTEGER NOT NULL,
            client_id TEXT NOT NULL,envelope_id TEXT NOT NULL,operation TEXT NOT NULL,operation_id TEXT,resources_json TEXT NOT NULL,
            digest BLOB NOT NULL,grant_revision INTEGER NOT NULL,decision TEXT NOT NULL,code TEXT NOT NULL);
INSERT INTO _migrations VALUES(4,'journal','2026-10-24');
CREATE TABLE door_port(
             singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
             port INTEGER NOT NULL CHECK(port BETWEEN 1 AND 65535));
INSERT INTO _migrations VALUES(5,'door_port','2026-10-24');
-- short_route_prefix has no rows to rewrite in this frozen empty schema.
INSERT INTO _migrations VALUES(6,'short_route_prefix','2026-10-24');
ALTER TABLE sessions RENAME TO single_sessions;
         CREATE TABLE sessions(
             client_id TEXT NOT NULL REFERENCES grants(client_id),
             session_id TEXT PRIMARY KEY,
             window_id TEXT NOT NULL,
             grant_revision INTEGER NOT NULL,
             next_client_sequence TEXT NOT NULL,
             next_server_sequence TEXT NOT NULL,
             ended_reason TEXT, ended_at_ms INTEGER, ended_limit INTEGER);
         INSERT INTO sessions SELECT client_id,session_id,window_id,grant_revision,
             next_client_sequence,next_server_sequence,NULL,NULL,NULL FROM single_sessions;
         DROP TABLE single_sessions;
         CREATE INDEX sessions_client ON sessions(client_id);
         ALTER TABLE operations ADD COLUMN session_id TEXT;
         ALTER TABLE operations ADD COLUMN grant_revision INTEGER;
         UPDATE operations SET grant_revision=(SELECT revision FROM grants WHERE grants.client_id=operations.client_id);
INSERT INTO _migrations VALUES(7,'multi_session','2026-10-24');
CREATE TABLE settings_designation(
        singleton INTEGER PRIMARY KEY CHECK(singleton=1),
        machine_id TEXT NOT NULL, client_id TEXT NOT NULL REFERENCES grants(client_id),
        public_key BLOB NOT NULL CHECK(length(public_key)=32));
     CREATE TABLE management_receipts(
        id TEXT PRIMARY KEY, client_id TEXT NOT NULL REFERENCES grants(client_id),
        operation TEXT NOT NULL, digest BLOB NOT NULL CHECK(length(digest)=32),
        grant_revision INTEGER NOT NULL, adopted_ms INTEGER NOT NULL, deadline_ms INTEGER NOT NULL,
        outcome TEXT NOT NULL);
     CREATE INDEX management_receipts_client ON management_receipts(client_id);
INSERT INTO _migrations VALUES(8,'management','2026-10-24');
CREATE TABLE device_last_use(
        client_id TEXT PRIMARY KEY REFERENCES grants(client_id),
        used_at_ms INTEGER NOT NULL CHECK(used_at_ms>=0 AND used_at_ms<=9007199254740991));
INSERT INTO _migrations VALUES(9,'device_last_use','2026-10-24');
CREATE TABLE cloud_transport(
        peer_id TEXT NOT NULL,
        kind TEXT NOT NULL CHECK(kind IN ('inbox','outbox','checkpoint')),
        id TEXT NOT NULL, digest BLOB CHECK(digest IS NULL OR length(digest)=32),
        carrier BLOB NOT NULL, response BLOB, refusal TEXT CHECK(refusal IS NULL OR (length(refusal) BETWEEN 1 AND 64 AND refusal NOT GLOB '*[^A-Z_]*')), created_ms INTEGER NOT NULL CHECK(created_ms BETWEEN 0 AND 9007199254740991),
        attempted INTEGER NOT NULL DEFAULT 0 CHECK(attempted IN (0,1)),
        PRIMARY KEY(peer_id,kind,id)); CREATE INDEX cloud_transport_expiry ON cloud_transport(kind,created_ms);
INSERT INTO _migrations VALUES(10,'cloud_transport','2026-10-24');
CREATE TABLE cloud_peers(
        project TEXT NOT NULL, client_id TEXT NOT NULL REFERENCES grants(client_id),
        owner TEXT NOT NULL, owner_key BLOB NOT NULL CHECK(length(owner_key)=32),
        generation INTEGER NOT NULL CHECK(generation>0),
        PRIMARY KEY(project,client_id));
     CREATE TABLE cloud_pair_receipts(
        offer TEXT PRIMARY KEY, digest BLOB NOT NULL CHECK(length(digest)=32),
        deadline_ms INTEGER NOT NULL, receipt BLOB NOT NULL, challenge TEXT NOT NULL,
        proof BLOB NOT NULL CHECK(length(proof)=32));
     CREATE TABLE cloud_peer_routes(
        project TEXT NOT NULL, uid TEXT NOT NULL,
        client_id TEXT NOT NULL REFERENCES grants(client_id),
        generation INTEGER NOT NULL CHECK(generation>0),
        revoked INTEGER NOT NULL CHECK(revoked IN(0,1)),
        pending INTEGER NOT NULL CHECK(pending IN(0,1)),
        PRIMARY KEY(project,uid));
     CREATE UNIQUE INDEX cloud_peer_live_route ON cloud_peer_routes(project,client_id) WHERE revoked=0;
     CREATE TABLE cloud_reroute_counters(machine_id TEXT PRIMARY KEY,generation INTEGER NOT NULL CHECK(generation>0));
     CREATE TABLE cloud_owner_projects(project TEXT PRIMARY KEY,config TEXT NOT NULL);
     CREATE TABLE cloud_remotes(name TEXT PRIMARY KEY,owner TEXT NOT NULL UNIQUE,connection TEXT NOT NULL);
INSERT INTO _migrations VALUES(11,'cloud_pairing','2026-10-24');
ALTER TABLE cloud_transport ADD COLUMN binding TEXT;
INSERT INTO _migrations VALUES(12,'cloud_final_binding','2026-10-24');
ALTER TABLE cloud_peers ADD COLUMN reroute_hash BLOB CHECK(reroute_hash IS NULL OR length(reroute_hash)=32);
INSERT INTO _migrations VALUES(13,'cloud_pair_tokens','2026-10-24');
