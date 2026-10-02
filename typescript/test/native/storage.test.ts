import path from 'node:path';
import Database from 'better-sqlite3';
import { describe, expect, it } from 'vitest';
import { resolveCliExecutables } from '../support/cli-executable.mjs';
import {
  expectError,
  parseWholeStdout,
  runCli,
  withSandbox,
  type Sandbox,
} from '../support/cli-process.js';
import {
  EXPECTED_NATIVE_SCHEMA_VERSION,
  FIXTURE_ATTEMPT_COUNT,
  FIXTURE_FINAL_BODY,
  FIXTURE_FINAL_REQUEST_ID,
  FIXTURE_IDENTITY_ID,
  seedStoragePrefix,
  seedStorageReference,
  storageSnapshot,
} from './storage-fixture.js';

// A development-only executable, never a hidden SQL command in the installed CLI.
if (!process.env.TMT_TEST_STORAGE_PROBE) {
  throw new Error('Select the native storage probe with TMT_TEST_STORAGE_PROBE.');
}
const { cli: probe } = resolveCliExecutables({
  TMT_TEST_CLI: process.env.TMT_TEST_STORAGE_PROBE,
});

function runStorage(sandbox: Sandbox, file = sandbox.database) {
  return runCli({ ...sandbox, cli: probe }, [file], { deadlineMs: 10_000 });
}

function migrationTimestamps(file: string): string[] {
  const database = new Database(file, { readonly: true });
  try {
    return database
      .prepare('SELECT applied_at FROM _migrations ORDER BY version')
      .pluck()
      .all() as string[];
  } finally {
    database.close();
  }
}

function table(snapshot: ReturnType<typeof storageSnapshot>, name: string) {
  const found = snapshot.tables.find((candidate) => candidate.name === name);
  expect(found, `Missing table ${name}`).toBeDefined();
  return found!;
}

function expectNativeSchema(
  reference: ReturnType<typeof storageSnapshot>,
  migrated: ReturnType<typeof storageSnapshot>
): void {
  expect(migrated.migrations.slice(0, 8)).toEqual(reference.migrations);
  expect(migrated.migrations).toHaveLength(EXPECTED_NATIVE_SCHEMA_VERSION);
  const additions = [
    { version: 9, name: 'add identity lifetimes and reusable retired names' },
    { version: 10, name: 'add durable identity retirement hooks' },
    { version: 11, name: 'add installation-owned local Office blocks' },
    { version: 12, name: 'add durable inbox routes and recipient attention' },
    { version: 13, name: 'add searchable identity metadata' },
    { version: 14, name: 'add local Office discussion board' },
    { version: 15, name: 'add identity-owned local Office presentation profiles' },
    { version: 16, name: 'add installation-owned local Office prop catalog' },
    { version: 17, name: 'add installation-owned local Office avatar catalog' },
    { version: 18, name: 'admit bounded directional Office prop packs' },
    { version: 19, name: 'add installation lobby target to local Office layouts' },
    { version: 20, name: 'allow bounded local Office placement customization' },
    { version: 21, name: 'add local whiteboard documents and operation receipts' },
    { version: 22, name: 'capture immutable whiteboard revisions and annotations' },
    { version: 23, name: 'retain Office request dispatch operation receipts' },
    { version: 24, name: 'add explicit local Office meeting membership' },
    { version: 25, name: 'distinguish inbox announcements from replyable requests' },
    { version: 26, name: 'retain original room context on durable requests' },
    { version: 27, name: 'index retained request conversations' },
    { version: 28, name: 'add atomic user-built Office world layouts' },
    { version: 29, name: 'add identity-owned expiring self-reported status' },
    { version: 30, name: 'extend shared Office discussions with room scopes' },
    { version: 31, name: 'retain retired meeting rooms without accepting new work' },
    { version: 32, name: 'track advisory wake attempts on durable inbox requests' },
    {
      version: 33,
      name: 'separate remembered harness preferences from binding runtime observations',
    },
    {
      version: 34,
      name: 'retain foreground launch ownership for binding runtime observations',
    },
    { version: 35, name: 'claim originator reply and detached timeout hints' },
    { version: 36, name: 'record extension storage cutovers and fence moved Office rows' },
    { version: 37, name: 'keep driver-owned resume state beside remembered sessions' },
    { version: 38, name: 'mark resume launches pending until a provider start confirms them' },
    {
      version: 39,
      name: 'admit a second terminal host in bindings, request fences and host servers',
    },
    { version: 40, name: 'advance one change cursor on every change to core-owned records' },
    {
      version: 41,
      name: 'admit any approved host driver in bindings, request fences and host servers',
    },
    {
      version: 42,
      name: "record the observed pane process incarnation beside each binding's pane pid",
    },
    { version: 43, name: 'record explicit automatic identity name provenance' },
    { version: 44, name: 'persist pane reply notice batches and one-shot worker claims' },
  ];
  expect(migrated.migrations.slice(8)).toEqual(additions);
  expect(migrated.tables.map(({ name }) => name)).toEqual(
    [
      ...reference.tables.map(({ name }) => name),
      'change_cursor',
      'extension_storage_cutovers',
      'host_servers',
      'identity_hooks',
      'identity_metadata',
      'identity_status',
      'identity_session_preferences',
      'office_local_blocks',
      'office_local_worlds',
      'office_local_profiles',
      'office_prop_catalog',
      'office_prop_packs',
      'office_avatar_catalog',
      'office_avatar_packs',
      'office_board_entries',
      'office_board_operations',
      'office_board_state',
      'office_dispatch_operations',
      'office_meeting_rooms',
      'office_meeting_members',
      'office_whiteboards',
      'office_whiteboard_operations',
      'office_whiteboard_snapshots',
      'office_whiteboard_snapshot_images',
      'request_recipient_attention_identities',
      'request_notifications',
      'reply_notice_batches',
      'reply_notices',
    ].sort()
  );
  expect(table(migrated, 'office_avatar_catalog').rows).toEqual([
    {
      singleton: 1,
      revision: 0,
      previous_kind: null,
      previous_digest: null,
      previous_base_revision: null,
      previous_result_revision: null,
    },
  ]);
  const avatars = table(migrated, 'office_avatar_packs');
  expect(avatars.rows).toEqual([]);
  expect(avatars.columns.map(({ name }) => name)).toEqual([
    'digest',
    'bytes',
    'avatar_count',
    'installed_revision',
    'installed_at_ms',
  ]);
  for (const [name, columns] of [
    [
      'identity_session_preferences',
      [
        'identity_id',
        'preferred_harness',
        'remembered_harness',
        'runtime_mode',
        'provider_session_id',
        'driver_state',
        'driver_state_version',
        'stale_at_ms',
        'resume_pending_at_ms',
      ],
    ],
    ['identity_status', ['identity_id', 'activity', 'mood', 'updated_at_ms', 'expires_at_ms']],
    [
      'office_local_worlds',
      [
        'singleton',
        'id',
        'created_at_ms',
        'layout_revision',
        'layout_json',
        'layout_updated_at_ms',
      ],
    ],
    ['office_dispatch_operations', ['operation_id', 'intent_digest', 'receipt']],
    ['office_meeting_rooms', ['room_id', 'name', 'revision', 'retired']],
    ['office_meeting_members', ['room_id', 'identity_id']],
    ['office_whiteboard_snapshot_images', ['snapshot_id', 'pixel_digest', 'png']],
    ['office_whiteboards', ['document_id', 'world_id', 'revision', 'scene', 'updated_at_ms']],
    [
      'office_whiteboard_operations',
      [
        'world_id',
        'operation_id',
        'intent_digest',
        'document_id',
        'revision',
        'changed',
        'updated_at_ms',
      ],
    ],
    [
      'office_whiteboard_snapshots',
      [
        'snapshot_id',
        'world_id',
        'intent_digest',
        'document_id',
        'document_revision',
        'scene',
        'selected_element_ids',
        'annotation',
        'created_at_ms',
      ],
    ],
  ] as const) {
    const resource = table(migrated, name);
    expect(resource.rows).toEqual([]);
    expect(resource.columns.map((column) => column.name)).toEqual(columns);
  }
  const snapshots = table(migrated, 'office_whiteboard_snapshots');
  const images = table(migrated, 'office_whiteboard_snapshot_images');
  expect(images.columns[0]).toMatchObject({ name: 'snapshot_id', type: 'TEXT', notnull: 1, pk: 1 });
  expect(images.foreignKeys).toEqual([
    {
      id: 0,
      seq: 0,
      table: 'office_whiteboard_snapshots',
      from: 'snapshot_id',
      to: 'snapshot_id',
      on_update: 'NO ACTION',
      on_delete: 'NO ACTION',
      match: 'NONE',
    },
  ]);
  expect(snapshots.columns[0]).toMatchObject({
    name: 'snapshot_id',
    type: 'TEXT',
    notnull: 1,
    pk: 1,
  });
  expect(snapshots.foreignKeys).toEqual([
    {
      id: 0,
      seq: 0,
      table: 'office_local_worlds',
      from: 'world_id',
      to: 'id',
      on_update: 'NO ACTION',
      on_delete: 'NO ACTION',
      match: 'NONE',
    },
  ]);
  expect(snapshots.indexes).toHaveLength(1);
  expect(snapshots.indexes[0]).toMatchObject({ unique: 1, origin: 'pk', partial: 0 });
  expect(
    snapshots.indexes[0]!.columns.filter((column) => column.key === 1).map((column) => column.name)
  ).toEqual(['snapshot_id']);
  const hooks = table(migrated, 'identity_hooks');
  expect(hooks.rows).toEqual([]);
  expect(hooks.columns).toEqual([
    { cid: 0, name: 'consumer', type: 'TEXT', notnull: 1, dflt_value: null, pk: 1 },
    { cid: 1, name: 'identity_id', type: 'TEXT', notnull: 1, dflt_value: null, pk: 2 },
    { cid: 2, name: 'reference', type: 'TEXT', notnull: 1, dflt_value: null, pk: 3 },
    { cid: 3, name: 'state', type: 'TEXT', notnull: 1, dflt_value: null, pk: 0 },
    { cid: 4, name: 'attempt_count', type: 'INTEGER', notnull: 1, dflt_value: '0', pk: 0 },
  ]);
  expect(hooks.foreignKeys).toEqual([
    {
      id: 0,
      seq: 0,
      table: 'identities',
      from: 'identity_id',
      to: 'id',
      on_update: 'NO ACTION',
      on_delete: 'NO ACTION',
      match: 'NONE',
    },
  ]);
  expect(hooks.indexes).toHaveLength(3);
  const queueIndex = hooks.indexes.find((index) => index.name === 'identity_hooks_pending');
  expect(queueIndex).toMatchObject({ unique: 0, partial: 0, origin: 'c' });
  expect(
    queueIndex?.columns.filter((column) => column.key === 1).map((column) => column.name)
  ).toEqual(['consumer', 'state', 'attempt_count', 'identity_id', 'reference']);
  const retirementIndex = hooks.indexes.find((index) => index.name === 'identity_hooks_retirement');
  expect(retirementIndex).toMatchObject({ unique: 0, partial: 0, origin: 'c' });
  expect(
    retirementIndex?.columns.filter((column) => column.key === 1).map((column) => column.name)
  ).toEqual(['identity_id', 'state']);

  const recipientAttention = table(migrated, 'request_recipient_attention_identities');
  expect(recipientAttention.rows).toEqual([]);
  expect(recipientAttention.columns.map(({ name }) => name)).toEqual([
    'identity_id',
    'latest_revision',
    'acknowledged_through',
  ]);

  const metadata = table(migrated, 'identity_metadata');
  expect(metadata.rows).toEqual([]);
  expect(
    metadata.columns.map(({ name, type, notnull, pk }) => ({ name, type, notnull, pk }))
  ).toEqual([
    { name: 'identity_id', type: 'TEXT', notnull: 1, pk: 1 },
    { name: 'key', type: 'TEXT', notnull: 1, pk: 2 },
    { name: 'value', type: 'TEXT', notnull: 1, pk: 0 },
  ]);
  expect(metadata.foreignKeys).toEqual([
    {
      id: 0,
      seq: 0,
      table: 'identities',
      from: 'identity_id',
      to: 'id',
      on_update: 'NO ACTION',
      on_delete: 'CASCADE',
      match: 'NONE',
    },
  ]);
  const metadataSearch = metadata.indexes.find(
    (index) => index.name === 'identity_metadata_search'
  );
  expect(metadataSearch).toMatchObject({ unique: 0, partial: 0, origin: 'c' });
  expect(
    metadataSearch?.columns.filter((column) => column.key === 1).map((column) => column.name)
  ).toEqual(['key', 'value', 'identity_id']);

  const unchangedTables = reference.tables
    .filter(
      ({ name }) =>
        name !== '_migrations' &&
        name !== 'identities' &&
        name !== 'bindings' &&
        name !== 'request_attempts' &&
        name !== 'request_responses'
    )
    .map(({ name }) => name);
  expect(
    migrated.tables.filter(({ name }) => unchangedTables.includes(name)).map(({ name }) => name)
  ).toEqual(unchangedTables);
  for (const name of unchangedTables) expect(table(migrated, name)).toEqual(table(reference, name));

  const oldBindings = table(reference, 'bindings');
  const newBindings = table(migrated, 'bindings');
  expect(newBindings.columns.slice(0, 11)).toEqual(oldBindings.columns);
  expect(newBindings.columns.slice(11)).toEqual([
    { cid: 11, name: 'runtime_state', type: 'TEXT', notnull: 1, dflt_value: "'unknown'", pk: 0 },
    { cid: 12, name: 'last_transition', type: 'TEXT', notnull: 0, dflt_value: null, pk: 0 },
    { cid: 13, name: 'runtime_pid', type: 'INTEGER', notnull: 0, dflt_value: null, pk: 0 },
    { cid: 14, name: 'runtime_start_identity', type: 'TEXT', notnull: 0, dflt_value: null, pk: 0 },
    {
      cid: 15,
      name: 'observed_provider_session_id',
      type: 'TEXT',
      notnull: 0,
      dflt_value: null,
      pk: 0,
    },
    { cid: 16, name: 'launch_owner_pid', type: 'INTEGER', notnull: 0, dflt_value: null, pk: 0 },
    {
      cid: 17,
      name: 'launch_owner_start_identity',
      type: 'TEXT',
      notnull: 0,
      dflt_value: null,
      pk: 0,
    },
    { cid: 18, name: 'pane_incarnation', type: 'TEXT', notnull: 0, dflt_value: null, pk: 0 },
  ]);
  expect(newBindings.indexes).toEqual(oldBindings.indexes);
  expect(newBindings.foreignKeys).toEqual(oldBindings.foreignKeys);
  expect(newBindings.rows).toEqual(
    oldBindings.rows.map((row) => ({
      ...row,
      runtime_state: 'unknown',
      last_transition: null,
      runtime_pid: null,
      runtime_start_identity: null,
      observed_provider_session_id: null,
      launch_owner_pid: null,
      launch_owner_start_identity: null,
      // Bindings made before schema 42 keep an unknown pane incarnation.
      pane_incarnation: null,
    }))
  );

  const oldHistory = table(reference, '_migrations');
  const newHistory = table(migrated, '_migrations');
  expect(newHistory.columns).toEqual(oldHistory.columns);
  expect(newHistory.indexes).toEqual(oldHistory.indexes);
  expect(newHistory.foreignKeys).toEqual(oldHistory.foreignKeys);
  expect(newHistory.rows).toEqual([...oldHistory.rows, ...additions]);
  const oldAttempts = table(reference, 'request_attempts');
  const newAttempts = table(migrated, 'request_attempts');
  expect(newAttempts.rows).toEqual(
    oldAttempts.rows.map((row) => ({
      ...row,
      request_kind: 'request',
      room_id: null,
      route_kind: 'pane',
      recipient_attention_revision: 0,
      recipient_attention_acknowledged_revision: 0,
      wake_state: 'not_attempted',
      host: null,
    }))
  );
  expect(newAttempts.columns.map(({ name }) => name)).toContain('route_kind');
  expect(newAttempts.columns.map(({ name }) => name)).toContain('request_kind');
  expect(newAttempts.columns.map(({ name }) => name)).toContain('room_id');
  expect(newAttempts.columns.find(({ name }) => name === 'wake_state')).toMatchObject({
    name: 'wake_state',
    type: 'TEXT',
    notnull: 1,
    dflt_value: "'not_attempted'",
    pk: 0,
  });
  for (const name of [
    'request_attempts_room_recipient_attention',
    'request_attempts_room_response_attention',
    'request_history_recipient',
    'request_history_recipient_room',
    'request_history_room',
  ]) {
    expect(newAttempts.indexes.some((index) => index.name === name)).toBe(true);
  }
  expect(newAttempts.columns.map(({ name }) => name)).toContain('recipient_attention_revision');
  expect(
    newAttempts.indexes.some(({ name }) => name === 'request_attempts_recipient_attention')
  ).toBe(true);

  const oldResponses = table(reference, 'request_responses');
  const newResponses = table(migrated, 'request_responses');
  expect(newResponses.rows).toEqual(
    oldResponses.rows.map((row) => ({
      ...row,
      route_kind: 'pane',
      route_recipient_identity_id: null,
      host: null,
    }))
  );
  expect(newResponses.columns.map(({ name }) => name)).toContain('route_kind');
  // Schema 39: a NULL host is tmux, so earlier fences are never rewritten.
  expect(newAttempts.columns.map(({ name }) => name)).toContain('host');
  expect(newResponses.columns.map(({ name }) => name)).toContain('host');

  const oldIdentities = table(reference, 'identities');
  const newIdentities = table(migrated, 'identities');
  expect(newIdentities.columns.slice(0, 5)).toEqual(oldIdentities.columns);
  expect(newIdentities.columns.slice(5)).toEqual([
    { cid: 5, name: 'lifetime', type: 'TEXT', notnull: 1, dflt_value: "'saved'", pk: 0 },
    { cid: 6, name: 'retired_at_ms', type: 'INTEGER', notnull: 0, dflt_value: null, pk: 0 },
    { cid: 7, name: 'auto_named', type: 'INTEGER', notnull: 1, dflt_value: '0', pk: 0 },
  ]);
  expect(newIdentities.rows).toEqual(
    oldIdentities.rows.map((row) => ({
      ...row,
      lifetime: 'saved',
      retired_at_ms: null,
      auto_named: 0,
    }))
  );
  expect(newIdentities.foreignKeys).toEqual(oldIdentities.foreignKeys);
  expect(newIdentities.indexes).toHaveLength(oldIdentities.indexes.length);
  expect(newIdentities.indexes.find((index) => index.origin === 'pk')).toEqual(
    oldIdentities.indexes.find((index) => index.origin === 'pk')
  );
  const activeNameIndex = newIdentities.indexes.find(
    (index) =>
      index.unique === 1 &&
      index.partial === 1 &&
      index.columns.some((column) => column.name === 'canonical_name' && column.key === 1)
  );
  expect(activeNameIndex).toMatchObject({ name: 'identities_active_name' });
  expect(
    newIdentities.indexes.some(
      (index) =>
        index.unique === 1 &&
        index.partial === 0 &&
        index.columns.some((column) => column.name === 'canonical_name' && column.key === 1)
    )
  ).toBe(false);
}

describe('native SQLite lifecycle compatibility', () => {
  it.each(Array.from({ length: 9 }, (_, version) => version))(
    'upgrades closed TypeScript schema %i to the current native schema without changing durable data',
    async (version) => {
      await withSandbox(async (sandbox) => {
        const reference = path.join(sandbox.root, 'reference', 'state.db');
        seedStorageReference(reference, version);
        seedStoragePrefix(sandbox.database, version);
        const originalTimestamps = migrationTimestamps(sandbox.database);

        const result = await runStorage(sandbox);
        expect(result.status).toBe(0);
        expect(parseWholeStdout(result)).toEqual({
          path: sandbox.database,
          open: true,
          schemaVersion: EXPECTED_NATIVE_SCHEMA_VERSION,
          journalMode: 'wal',
          foreignKeys: true,
          busyTimeoutMs: 5000,
          synchronous: 'normal',
          fts5: true,
        });
        const migrated = storageSnapshot(sandbox.database);
        expectNativeSchema(storageSnapshot(reference), migrated);
        if (version >= 5) {
          const responses = migrated.tables.find(
            (table) => table.name === 'request_responses'
          )!.rows;
          expect(responses).toHaveLength(FIXTURE_ATTEMPT_COUNT);
          expect(
            responses.find((row) => row.request_id === FIXTURE_FINAL_REQUEST_ID)
          ).toMatchObject({
            body: FIXTURE_FINAL_BODY,
            body_bytes: Buffer.byteLength(FIXTURE_FINAL_BODY),
            response_expires_at_ms: 1_700_604_800_200,
          });
          expect(responses.find((row) => row.attempt_id === '')).toHaveProperty(
            'response_expires_at_ms',
            Number.MAX_SAFE_INTEGER
          );
        }
        if (version === 7) {
          const attempts = migrated.tables.find((table) => table.name === 'request_attempts')!.rows;
          expect(attempts.find((row) => row.attempt_id === '')).toMatchObject({
            originator_identity_id: FIXTURE_IDENTITY_ID,
            attention_revision: 69,
            attention_acknowledged_revision: 0,
          });
          expect(attempts.find((row) => row.attempt_id === 'attempt-003')).toHaveProperty(
            'attention_revision',
            1
          );
        }
        const timestamps = migrationTimestamps(sandbox.database);
        expect(timestamps).toHaveLength(EXPECTED_NATIVE_SCHEMA_VERSION);
        expect(timestamps.slice(0, version)).toEqual(originalTimestamps);
        for (const timestamp of timestamps) {
          expect(timestamp).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/);
        }
        expect((await runStorage(sandbox)).status).toBe(0);
        expect(storageSnapshot(sandbox.database)).toEqual(migrated);
        expect(migrationTimestamps(sandbox.database)).toEqual(timestamps);
      });
    }
  );

  it('converges concurrent native processes on an existing historical WAL database', async () => {
    await withSandbox(async (sandbox) => {
      const reference = path.join(sandbox.root, 'reference', 'state.db');
      seedStorageReference(reference, 8);
      seedStoragePrefix(sandbox.database, 8);
      // Wait for every bounded child before the sandbox can be removed, even
      // when an individual launch fails. No child may escape failed assertions.
      const results = await Promise.allSettled(
        Array.from({ length: 4 }, () => runStorage(sandbox))
      );
      for (const result of results) {
        expect(result.status).toBe('fulfilled');
        if (result.status === 'fulfilled') {
          expect(result.value.status, result.value.stdout).toBe(0);
          expect(parseWholeStdout(result.value).schemaVersion).toBe(EXPECTED_NATIVE_SCHEMA_VERSION);
        }
      }
      expectNativeSchema(storageSnapshot(reference), storageSnapshot(sandbox.database));
    });
  });

  it('enforces retirement subscription uniqueness, foreign keys and delivery state bounds', async () => {
    await withSandbox(async (sandbox) => {
      expect((await runStorage(sandbox)).status).toBe(0);
      const database = new Database(sandbox.database);
      try {
        database.pragma('foreign_keys = ON');
        database.exec(
          "INSERT INTO identities (id, name, canonical_name, created_at, updated_at) VALUES ('hook-id', 'Agent', 'agent', 'created', 'updated')"
        );
        const insert = database.prepare(
          'INSERT INTO identity_hooks (consumer, identity_id, reference, state, attempt_count) VALUES (?, ?, ?, ?, ?)'
        );
        insert.run('office', 'hook-id', 'scope', 'registered', 0);
        expect(() => insert.run('office', 'hook-id', 'scope', 'pending', 0)).toThrow(/UNIQUE/);
        expect(() => insert.run('office', 'missing', 'scope', 'registered', 0)).toThrow(
          /FOREIGN KEY/
        );
        for (const state of ['unknown', '']) {
          expect(() => insert.run('office', 'hook-id', 'other', state, 0)).toThrow(/CHECK/);
        }
        for (const attempt of [-1, 0.5]) {
          expect(() => insert.run('office', 'hook-id', 'other', 'pending', attempt)).toThrow(
            /CHECK/
          );
        }
        expect(() => insert.run('', 'hook-id', 'other', 'registered', 0)).toThrow(/CHECK/);
        expect(() => insert.run('a'.repeat(65), 'hook-id', 'other', 'registered', 0)).toThrow(
          /CHECK/
        );
        expect(() => insert.run('office', 'hook-id', '', 'registered', 0)).toThrow(/CHECK/);
        expect(() => insert.run('office', 'hook-id', 'a'.repeat(257), 'registered', 0)).toThrow(
          /CHECK/
        );
        // Different consumers and references remain separate subscriptions.
        insert.run('another', 'hook-id', 'scope', 'pending', 1);
        insert.run('office', 'hook-id', 'other', 'delivered', 2);
        expect(
          database
            .prepare(
              'SELECT consumer, reference, state, attempt_count FROM identity_hooks ORDER BY consumer, reference'
            )
            .all()
        ).toEqual([
          { consumer: 'another', reference: 'scope', state: 'pending', attempt_count: 1 },
          { consumer: 'office', reference: 'other', state: 'delivered', attempt_count: 2 },
          { consumer: 'office', reference: 'scope', state: 'registered', attempt_count: 0 },
        ]);
        expect(() => database.exec("DELETE FROM identities WHERE id = 'hook-id'")).toThrow(
          /FOREIGN KEY/
        );
      } finally {
        database.close();
      }
    });
  });

  it('enforces schema constraints and preserves exchange rows when an identity is removed', async () => {
    await withSandbox(async (sandbox) => {
      expect((await runStorage(sandbox)).status).toBe(0);
      const database = new Database(sandbox.database);
      try {
        database.pragma('foreign_keys = ON');
        database.exec(
          "INSERT INTO identities (id, name, canonical_name, created_at, updated_at) VALUES ('id', 'Alice', 'alice', 'created', 'updated')"
        );
        expect(
          database.prepare('SELECT lifetime, retired_at_ms FROM identities WHERE id = ?').get('id')
        ).toEqual({
          lifetime: 'saved',
          retired_at_ms: null,
        });
        expect(() =>
          database.exec(
            "INSERT INTO identities (id, name, canonical_name, created_at, updated_at) VALUES ('other', 'ALICE', 'alice', 'created', 'updated')"
          )
        ).toThrow(/UNIQUE/);
        expect(() =>
          database.exec(
            "INSERT INTO identities (id, name, canonical_name, created_at, updated_at, lifetime, retired_at_ms) VALUES ('invalid-lifetime', 'Invalid', 'invalid-lifetime', 'created', 'updated', 'permanent', NULL)"
          )
        ).toThrow(/CHECK/);
        expect(() =>
          database.exec(
            "INSERT INTO identities (id, name, canonical_name, created_at, updated_at, lifetime, retired_at_ms) VALUES ('invalid-retired-zero', 'Invalid', 'invalid-retired-zero', 'created', 'updated', 'saved', 0)"
          )
        ).toThrow(/CHECK/);
        expect(() =>
          database.exec(
            "INSERT INTO identities (id, name, canonical_name, created_at, updated_at, lifetime, retired_at_ms) VALUES ('invalid-retired-large', 'Invalid', 'invalid-retired-large', 'created', 'updated', 'saved', 9007199254740992)"
          )
        ).toThrow(/CHECK/);
        database.exec(
          "UPDATE identities SET lifetime = 'temporary', retired_at_ms = 9007199254740991 WHERE id = 'id'"
        );
        database.exec(
          "INSERT INTO identities (id, name, canonical_name, created_at, updated_at, lifetime, retired_at_ms) VALUES ('new-id', 'ALICE', 'alice', 'created-new', 'updated-new', 'saved', NULL)"
        );
        expect(
          database.prepare('SELECT id, lifetime, retired_at_ms FROM identities ORDER BY id').all()
        ).toEqual([
          { id: 'id', lifetime: 'temporary', retired_at_ms: Number.MAX_SAFE_INTEGER },
          { id: 'new-id', lifetime: 'saved', retired_at_ms: null },
        ]);
        expect(() =>
          database.exec("INSERT INTO role_profiles VALUES ('missing', 'body', 'updated')")
        ).toThrow(/FOREIGN KEY/);
        expect(() => database.exec("INSERT INTO preamble_counters VALUES ('id', -1, 1)")).toThrow(
          /CHECK/
        );
        expect(() =>
          database.exec("INSERT INTO request_attention_identities VALUES ('id', 1, 2)")
        ).toThrow(/CHECK/);
        expect(() =>
          database.exec(
            "INSERT INTO request_attention_identities VALUES ('id', 9007199254740992, 0)"
          )
        ).toThrow(/CHECK/);
        database.exec(`
          INSERT INTO role_profiles VALUES ('id', 'body', 'updated');
          INSERT INTO request_attempts (
            attempt_id, request_id, identity_id, server_id, socket_path, server_pid,
            server_start_time, pane_id, pane_pid, wait_active, status,
            inject_preamble, cadence_reserved, prepared_at_ms, expires_at_ms,
            originator_kind, originator_identity_id, attention_revision
          ) VALUES ('attempt', 'request', 'id', 'server', '/socket', 1, 'start', '%1',
            2, 0, 'sent', 0, 0, 1, 2, 'explicit', 'id', 1);
          INSERT INTO request_responses (
            request_id, attempt_id, server_id, socket_path, server_pid, server_start_time,
            pane_id, pane_pid, body, body_bytes, submitted_at_ms
          ) VALUES ('request', 'attempt', 'server', '/socket', 1, 'start', '%1', 2, 'done', 4, 2);
          INSERT INTO request_attention_identities VALUES ('id', 1, 0);
          DELETE FROM identities WHERE id = 'id';
        `);
        expect(database.prepare('SELECT * FROM role_profiles').all()).toEqual([]);
        expect(
          database
            .prepare(
              'SELECT identity_id, originator_identity_id, attention_revision FROM request_attempts'
            )
            .get()
        ).toEqual({
          identity_id: null,
          originator_identity_id: 'id',
          attention_revision: 1,
        });
        expect(database.prepare('SELECT body, body_bytes FROM request_responses').get()).toEqual({
          body: 'done',
          body_bytes: 4,
        });
        expect(database.prepare('SELECT * FROM request_attention_identities').get()).toEqual({
          identity_id: 'id',
          latest_revision: 1,
          acknowledged_through: 0,
        });
      } finally {
        database.close();
      }
    });
  });

  it('rolls back schema 9 identity replacement when recording the migration fails', async () => {
    await withSandbox(async (sandbox) => {
      seedStoragePrefix(sandbox.database, 8);
      const before = storageSnapshot(sandbox.database);
      const writer = new Database(sandbox.database);
      try {
        writer.exec(`
          CREATE TRIGGER fail_schema9_history
          BEFORE INSERT ON _migrations
          WHEN NEW.version = 9
          BEGIN
            SELECT RAISE(ABORT, 'schema 9 history failure');
          END;
        `);
      } finally {
        writer.close();
      }

      const failed = await runStorage(sandbox);
      expect(failed.status).toBe(1);
      expect(expectError(failed, 'migration').error).toMatchObject({ migrationVersion: 9 });
      expect(storageSnapshot(sandbox.database)).toEqual(before);
      const afterFailure = new Database(sandbox.database, { readonly: true });
      try {
        expect(
          afterFailure
            .prepare("SELECT name FROM sqlite_schema WHERE type = 'trigger' AND name = ?")
            .get('fail_schema9_history')
        ).toEqual({ name: 'fail_schema9_history' });
      } finally {
        afterFailure.close();
      }

      const repair = new Database(sandbox.database);
      try {
        repair.exec('DROP TRIGGER fail_schema9_history');
      } finally {
        repair.close();
      }
      expect((await runStorage(sandbox)).status).toBe(0);
      expect(parseWholeStdout(await runStorage(sandbox))).toMatchObject({
        schemaVersion: EXPECTED_NATIVE_SCHEMA_VERSION,
      });
    });
  });

  it('rejects an invalid old foreign key without replacing the identity table', async () => {
    await withSandbox(async (sandbox) => {
      seedStoragePrefix(sandbox.database, 8);
      const writer = new Database(sandbox.database);
      try {
        writer.pragma('foreign_keys = OFF');
        writer
          .prepare('INSERT INTO role_profiles (identity_id, content, updated_at) VALUES (?, ?, ?)')
          .run('missing-old-identity', 'orphan role', 'updated');
      } finally {
        writer.close();
      }
      const before = storageSnapshot(sandbox.database);
      const result = await runStorage(sandbox);
      expect(result.status).toBe(1);
      expect(expectError(result, 'migration').error).toMatchObject({ migrationVersion: 9 });
      expect(storageSnapshot(sandbox.database)).toEqual(before);
    });
  });

  it('rejects an unexpected identity index without replacing the identity table', async () => {
    await withSandbox(async (sandbox) => {
      seedStoragePrefix(sandbox.database, 8);
      const writer = new Database(sandbox.database);
      try {
        writer.exec('CREATE INDEX identities_unexpected_name ON identities(name)');
      } finally {
        writer.close();
      }
      const before = storageSnapshot(sandbox.database);
      const result = await runStorage(sandbox);
      expect(result.status).toBe(1);
      expect(expectError(result, 'migration').error).toMatchObject({ migrationVersion: 9 });
      expect(storageSnapshot(sandbox.database)).toEqual(before);
    });
  });

  it('preserves old UUID provenance and gives a reused name no dependent rows', async () => {
    await withSandbox(async (sandbox) => {
      seedStoragePrefix(sandbox.database, 8);
      expect((await runStorage(sandbox)).status).toBe(0);
      const before = storageSnapshot(sandbox.database);
      const database = new Database(sandbox.database);
      try {
        database.pragma('foreign_keys = ON');
        database
          .prepare("UPDATE identities SET lifetime = 'temporary', retired_at_ms = ? WHERE id = ?")
          .run(1_700_000_000_999, FIXTURE_IDENTITY_ID);
        database
          .prepare(
            `INSERT INTO identities (
               id, name, canonical_name, created_at, updated_at, lifetime, retired_at_ms
             ) VALUES (?, ?, ?, ?, ?, ?, ?)`
          )
          .run(
            'identity-new',
            'Known Fixture',
            'known-fixture',
            '2026-02-01T00:00:00.000Z',
            '2026-02-01T00:00:00.000Z',
            'saved',
            null
          );

        expect(
          database
            .prepare(
              `SELECT DISTINCT identity_id, originator_identity_id, recipient_identity_id
               FROM request_attempts
               WHERE identity_id = ? OR originator_identity_id = ? OR recipient_identity_id = ?`
            )
            .all(FIXTURE_IDENTITY_ID, FIXTURE_IDENTITY_ID, FIXTURE_IDENTITY_ID)
        ).not.toHaveLength(0);
        expect(
          database
            .prepare(
              `SELECT COUNT(*) AS count FROM request_attempts
               WHERE identity_id = ? OR originator_identity_id = ? OR recipient_identity_id = ?`
            )
            .get('identity-new', 'identity-new', 'identity-new')
        ).toEqual({ count: 0 });
        for (const tableName of [
          'bindings',
          'identity_preambles',
          'preamble_counters',
          'request_attention_identities',
          'role_profiles',
        ]) {
          expect(
            database
              .prepare(`SELECT COUNT(*) AS count FROM ${tableName} WHERE identity_id = ?`)
              .get('identity-new')
          ).toEqual({ count: 0 });
        }
        expect(
          database.prepare('SELECT id FROM bindings WHERE identity_id = ?').get(FIXTURE_IDENTITY_ID)
        ).toEqual({ id: 'binding-known' });
        expect(
          database
            .prepare(
              'SELECT identity_id FROM request_attempts WHERE originator_identity_id = ? LIMIT 1'
            )
            .get(FIXTURE_IDENTITY_ID)
        ).toEqual({ identity_id: FIXTURE_IDENTITY_ID });
      } finally {
        database.close();
      }
      const after = storageSnapshot(sandbox.database);
      // Only identities changed, and the change cursor counted both writes.
      const changed = new Set(['identities', 'change_cursor']);
      expect(after.tables.filter(({ name }) => !changed.has(name))).toEqual(
        before.tables.filter(({ name }) => !changed.has(name))
      );
      expect(table(after, 'change_cursor').rows).toEqual([
        { id: 1, value: (table(before, 'change_cursor').rows[0] as { value: number }).value + 2 },
      ]);
      const identities = new Database(sandbox.database, { readonly: true });
      try {
        expect(
          identities
            .prepare(
              'SELECT id, lifetime, retired_at_ms FROM identities WHERE canonical_name = ? ORDER BY id'
            )
            .all('known-fixture')
        ).toEqual([
          { id: FIXTURE_IDENTITY_ID, lifetime: 'temporary', retired_at_ms: 1_700_000_000_999 },
          { id: 'identity-new', lifetime: 'saved', retired_at_ms: null },
        ]);
      } finally {
        identities.close();
      }
    });
  });

  it('preserves a user table that conflicts with the migration replacement name', async () => {
    await withSandbox(async (sandbox) => {
      seedStoragePrefix(sandbox.database, 8);
      const writer = new Database(sandbox.database);
      try {
        writer.exec(`
          CREATE TABLE identities_with_lifetime (note TEXT NOT NULL);
          INSERT INTO identities_with_lifetime VALUES ('user-owned content');
        `);
      } finally {
        writer.close();
      }
      const before = storageSnapshot(sandbox.database);
      const result = await runStorage(sandbox);
      expect(result.status).toBe(1);
      expect(expectError(result, 'migration').error).toMatchObject({ migrationVersion: 9 });
      expect(storageSnapshot(sandbox.database)).toEqual(before);
    });
  });

  it('rejects a malformed history table without replacing it', async () => {
    await withSandbox(async (sandbox) => {
      seedStoragePrefix(sandbox.database, 0);
      const writer = new Database(sandbox.database);
      try {
        writer.exec(
          'DROP TABLE _migrations; CREATE TABLE _migrations (version TEXT PRIMARY KEY, label TEXT NOT NULL)'
        );
      } finally {
        writer.close();
      }
      const result = await runStorage(sandbox);
      expect(result.status).toBe(1);
      expectError(result, 'incompatible-schema');
      const verification = new Database(sandbox.database, { readonly: true });
      try {
        expect(verification.pragma('table_info(_migrations)')).toMatchObject([
          { name: 'version', type: 'TEXT' },
          { name: 'label', type: 'TEXT' },
        ]);
        expect(verification.prepare('SELECT * FROM _migrations').all()).toEqual([]);
        expect(
          verification.prepare("SELECT name FROM sqlite_master WHERE type = 'table'").all()
        ).toEqual([{ name: '_migrations' }]);
      } finally {
        verification.close();
      }
    });
  });

  it('does not guess migration history for existing domain tables', async () => {
    await withSandbox(async (sandbox) => {
      seedStoragePrefix(sandbox.database, 8);
      const writer = new Database(sandbox.database);
      let identities: unknown;
      let responses: unknown;
      try {
        identities = writer.prepare('SELECT * FROM identities ORDER BY id').all();
        responses = writer.prepare('SELECT * FROM request_responses ORDER BY request_id').all();
        writer.exec('DROP TABLE _migrations');
      } finally {
        writer.close();
      }
      const result = await runStorage(sandbox);
      expect(result.status).toBe(1);
      expect(expectError(result, 'migration').error).toMatchObject({ migrationVersion: 1 });
      const verification = new Database(sandbox.database, { readonly: true });
      try {
        expect(verification.prepare('SELECT * FROM identities ORDER BY id').all()).toEqual(
          identities
        );
        expect(
          verification.prepare('SELECT * FROM request_responses ORDER BY request_id').all()
        ).toEqual(responses);
        // As in TypeScript, history-table initialization may commit, but no
        // version can be fabricated to repair an unsupported existing schema.
        expect(verification.prepare('SELECT * FROM _migrations').all()).toEqual([]);
      } finally {
        verification.close();
      }
    });
  });

  it.each(['renamed', 'gap', 'future'])(
    'rejects %s migration history without resetting or rewriting data',
    async (kind) => {
      await withSandbox(async (sandbox) => {
        seedStoragePrefix(sandbox.database, 8);
        if (kind === 'future') {
          const upgraded = await runStorage(sandbox);
          expect(upgraded.status).toBe(0);
          expect(parseWholeStdout(upgraded)).toMatchObject({
            schemaVersion: EXPECTED_NATIVE_SCHEMA_VERSION,
          });
        }
        const writer = new Database(sandbox.database);
        try {
          if (kind === 'renamed')
            writer.exec("UPDATE _migrations SET name = 'unknown' WHERE version = 2");
          else if (kind === 'gap') writer.exec('DELETE FROM _migrations WHERE version = 2');
          else
            writer
              .prepare("INSERT INTO _migrations VALUES (?, 'future', 'timestamp')")
              .run(EXPECTED_NATIVE_SCHEMA_VERSION + 1);
        } finally {
          writer.close();
        }
        const before = storageSnapshot(sandbox.database);
        const result = await runStorage(sandbox);
        expect(result.status).toBe(1);
        expectError(result, 'incompatible-schema');
        expect(storageSnapshot(sandbox.database)).toEqual(before);
      });
    }
  );

  it('rolls back migration 6 on an invalid historical anchor and recovers after explicit repair', async () => {
    await withSandbox(async (sandbox) => {
      seedStoragePrefix(sandbox.database, 5);
      const writer = new Database(sandbox.database);
      try {
        writer.exec('UPDATE request_attempts SET prepared_at_ms = 0');
      } finally {
        writer.close();
      }
      const before = storageSnapshot(sandbox.database);
      const result = await runStorage(sandbox);
      expect(result.status).toBe(1);
      expect(expectError(result, 'migration').error).toMatchObject({
        migrationVersion: 6,
        retryable: false,
      });
      expect(storageSnapshot(sandbox.database)).toEqual(before);
      const repair = new Database(sandbox.database);
      try {
        repair.exec('UPDATE request_attempts SET prepared_at_ms = 1700000000000');
      } finally {
        repair.close();
      }
      expect((await runStorage(sandbox)).status).toBe(0);
    });
  });

  it('reports bounded writer contention and leaves storage usable after lock release', async () => {
    await withSandbox(async (sandbox) => {
      const reference = path.join(sandbox.root, 'reference', 'state.db');
      seedStorageReference(reference, 8);
      seedStoragePrefix(sandbox.database, 8);
      const before = storageSnapshot(sandbox.database);
      const writer = new Database(sandbox.database);
      try {
        writer.exec('BEGIN IMMEDIATE');
        const result = await runStorage(sandbox);
        expect(result.status).toBe(1);
        expect(expectError(result, 'busy').error).toMatchObject({ retryable: true });
      } finally {
        if (writer.inTransaction) writer.exec('ROLLBACK');
        writer.close();
      }
      expect(storageSnapshot(sandbox.database)).toEqual(before);
      expect((await runStorage(sandbox)).status).toBe(0);
      expectNativeSchema(storageSnapshot(reference), storageSnapshot(sandbox.database));
    });
  }, 15_000);
});
