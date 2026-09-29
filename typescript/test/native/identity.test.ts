import Database from 'better-sqlite3';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { calibrateTmuxTripwire } from './tmux-tripwire.js';
import { EXPECTED_NATIVE_SCHEMA_VERSION } from './storage-fixture.js';
import {
  expectError,
  expectJsonSuccess,
  parseWholeStdout,
  runCli,
  type Sandbox,
  withSandbox,
} from '../support/cli-process.js';

type Identity = {
  readonly id: string;
  readonly name: string;
  readonly canonicalName: string;
  readonly lifetime: 'temporary' | 'saved';
};

const TEMPORARY_ID = '11111111-1111-4111-8111-111111111111';
const TEMPORARY_BINDING_ID = '11111111-1111-4111-8111-111111111112';
const TEMPORARY_SERVER_ID = '11111111-1111-4111-8111-111111111113';
const RETIRED_ID = '22222222-2222-4222-8222-222222222222';
const RETIRED_BINDING_ID = '22222222-2222-4222-8222-222222222223';
const RETIRED_SERVER_ID = '22222222-2222-4222-8222-222222222224';

function assertIdentity(value: unknown): asserts value is Identity {
  expect(value).toEqual({
    id: expect.any(String),
    name: expect.any(String),
    canonicalName: expect.any(String),
    lifetime: expect.stringMatching(/^(temporary|saved)$/),
  });
  expect(Object.keys(value as object).sort()).toEqual(['canonicalName', 'id', 'lifetime', 'name']);
  expect((value as Identity).id).toMatch(
    /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i
  );
}

function documentIdentity(result: Awaited<ReturnType<typeof runCli>>): Identity {
  expect(result.status).toBe(0);
  expect(result.stderr).toBe('');
  const document = parseWholeStdout(result) as { identity?: unknown };
  assertIdentity(document.identity);
  return document.identity;
}

async function initializeSchema(sandbox: Sandbox): Promise<void> {
  expectJsonSuccess(await runCli(sandbox, ['identity', 'list', '--json']), { identities: [] });
  expect(existsSync(sandbox.database)).toBe(true);
}

function withDatabase<T>(file: string, callback: (database: Database.Database) => T): T {
  const database = new Database(file);
  try {
    database.pragma('foreign_keys = ON');
    return callback(database);
  } finally {
    database.close();
  }
}

function seedIdentity(
  database: Database.Database,
  identity: {
    readonly id: string;
    readonly name: string;
    readonly canonicalName: string;
    readonly lifetime: 'temporary' | 'saved';
    readonly retiredAtMs?: number;
  }
): void {
  database
    .prepare(
      `INSERT INTO identities
         (id, name, canonical_name, created_at, updated_at, lifetime, retired_at_ms)
       VALUES (?, ?, ?, ?, ?, ?, ?)`
    )
    .run(
      identity.id,
      identity.name,
      identity.canonicalName,
      '2026-01-01T00:00:00.000Z',
      '2026-01-02T00:00:00.000Z',
      identity.lifetime,
      identity.retiredAtMs ?? null
    );
}

function seedDependents(database: Database.Database, identityId: string, suffix: string): void {
  database
    .prepare('INSERT INTO role_profiles (identity_id, content, updated_at) VALUES (?, ?, ?)')
    .run(identityId, `role-${suffix}`, '2026-01-03T00:00:00.000Z');
  database
    .prepare('INSERT INTO identity_preambles (identity_id, content, updated_at) VALUES (?, ?, ?)')
    .run(identityId, `preamble-${suffix}`, '2026-01-04T00:00:00.000Z');
  database
    .prepare(
      `INSERT INTO bindings (
         id, identity_id, transport, pane_id, server_id, socket_path, server_pid,
         server_start_time, pane_pid, bound_at, last_verified_at
       ) VALUES (?, ?, 'tmux', ?, ?, ?, 101, ?, 202, ?, ?)`
    )
    .run(
      suffix === 'temporary' ? TEMPORARY_BINDING_ID : RETIRED_BINDING_ID,
      identityId,
      `%${suffix === 'temporary' ? '12' : '13'}`,
      suffix === 'temporary' ? TEMPORARY_SERVER_ID : RETIRED_SERVER_ID,
      `/tmp/${suffix}.sock`,
      `start-${suffix}`,
      '2026-01-05T00:00:00.000Z',
      '2026-01-06T00:00:00.000Z'
    );
}

function dependentSnapshot(
  database: Database.Database,
  identityId: string
): {
  readonly role: unknown;
  readonly preamble: unknown;
  readonly binding: unknown;
} {
  return {
    role: database
      .prepare('SELECT identity_id, content, updated_at FROM role_profiles WHERE identity_id = ?')
      .get(identityId),
    preamble: database
      .prepare(
        'SELECT identity_id, content, updated_at FROM identity_preambles WHERE identity_id = ?'
      )
      .get(identityId),
    binding: database
      .prepare(
        `SELECT id, identity_id, transport, pane_id, server_id, socket_path, server_pid,
                server_start_time, pane_pid, bound_at, last_verified_at
           FROM bindings WHERE identity_id = ?`
      )
      .get(identityId),
  };
}

describe('native durable identity process boundary', () => {
  it('keeps create/show/list exact across restarts and directories', async () => {
    await withSandbox(async (sandbox) => {
      const globalConfig = '{ unrelated malformed global config';
      const localConfig = '{ unrelated malformed local config';
      mkdirSync(sandbox.globalDir, { recursive: true });
      writeFileSync(sandbox.globalConfig, globalConfig);
      writeFileSync(sandbox.localConfig, localConfig);

      const createdResult = await runCli(sandbox, ['identity', 'create', '  Alice  ', '--json']);
      expect(createdResult.status).toBe(0);
      expect(createdResult.stderr).toBe('');
      const created = documentIdentity(createdResult);
      expect(created).toMatchObject({
        name: 'Alice',
        canonicalName: 'alice',
        lifetime: 'saved',
      });
      expect(parseWholeStdout(createdResult)).toEqual({ identity: created, created: true });

      const nested = path.join(sandbox.cwd, 'nested', 'directory');
      mkdirSync(nested, { recursive: true });
      const otherDirectory = { ...sandbox, cwd: nested };
      const repeated = await runCli(otherDirectory, ['identity', 'create', 'ALICE', '--json']);
      expectJsonSuccess(repeated, { identity: created, created: false });
      expectJsonSuccess(await runCli(otherDirectory, ['identity', 'show', 'alice', '--json']), {
        identity: created,
      });
      expectJsonSuccess(await runCli(sandbox, ['identity', 'list', '--json']), {
        identities: [created],
      });
      expect(readFileSync(sandbox.globalConfig, 'utf8')).toBe(globalConfig);
      expect(readFileSync(sandbox.localConfig, 'utf8')).toBe(localConfig);
    });
  });

  it('reuses Unicode canonical names and exposes only the public identity fields', async () => {
    await withSandbox(async (sandbox) => {
      const firstResult = await runCli(sandbox, ['identity', 'create', 'Ｇｅｍｉｎｉ', '--json']);
      const first = documentIdentity(firstResult);
      expect(first).toEqual({
        id: first.id,
        name: 'Ｇｅｍｉｎｉ',
        canonicalName: 'gemini',
        lifetime: 'saved',
      });
      expect(parseWholeStdout(firstResult)).toEqual({ identity: first, created: true });

      const repeatedResult = await runCli(sandbox, ['identity', 'create', 'GEMINI', '--json']);
      expectJsonSuccess(repeatedResult, { identity: first, created: false });
      const shownResult = await runCli(sandbox, ['identity', 'show', 'ＧＥＭＩＮＩ', '--json']);
      expectJsonSuccess(shownResult, { identity: first });
      expect(
        Object.keys((parseWholeStdout(shownResult) as { identity: object }).identity).sort()
      ).toEqual(['canonicalName', 'id', 'lifetime', 'name']);
    });
  });

  it('promotes a temporary row in place while retaining bindings, roles, and preambles', async () => {
    await withSandbox(async (sandbox) => {
      await initializeSchema(sandbox);
      const temporary = {
        id: TEMPORARY_ID,
        name: 'Temporary Name',
        canonicalName: 'temporary name',
        lifetime: 'temporary' as const,
      };
      const before = withDatabase(sandbox.database, (database) => {
        seedIdentity(database, temporary);
        seedDependents(database, temporary.id, 'temporary');
        return {
          identity: database.prepare('SELECT * FROM identities WHERE id = ?').get(temporary.id),
          dependents: dependentSnapshot(database, temporary.id),
        };
      });

      const promoted = await runCli(sandbox, ['identity', 'create', 'TEMPORARY NAME', '--json']);
      expectJsonSuccess(promoted, {
        identity: { ...temporary, lifetime: 'saved' },
        created: false,
      });
      const afterPromotion = withDatabase(sandbox.database, (database) => ({
        identity: database.prepare('SELECT * FROM identities WHERE id = ?').get(temporary.id),
        dependents: dependentSnapshot(database, temporary.id),
      }));
      expect(afterPromotion.identity).toEqual({
        ...(before.identity as Record<string, unknown>),
        lifetime: 'saved',
        updated_at: expect.any(String),
      });
      expect((afterPromotion.identity as Record<string, unknown>).updated_at).not.toBe(
        (before.identity as Record<string, unknown>).updated_at
      );
      expect(afterPromotion.dependents).toEqual(before.dependents);
      const repeated = await runCli(sandbox, ['identity', 'create', 'temporary name', '--json']);
      expectJsonSuccess(repeated, {
        identity: { ...temporary, lifetime: 'saved' },
        created: false,
      });
      expect(
        withDatabase(sandbox.database, (database) =>
          database.prepare('SELECT * FROM identities WHERE id = ?').get(temporary.id)
        )
      ).toEqual(afterPromotion.identity);
    });
  });

  it('omits retired identities and gives a reused name a fresh UUID without dependent rows', async () => {
    await withSandbox(async (sandbox) => {
      await initializeSchema(sandbox);
      const retired = {
        id: RETIRED_ID,
        name: 'Reusable',
        canonicalName: 'reusable',
        lifetime: 'saved' as const,
        retiredAtMs: 1_700_000_000_000,
      };
      const before = withDatabase(sandbox.database, (database) => {
        seedIdentity(database, retired);
        seedDependents(database, retired.id, 'retired');
        return {
          identity: database.prepare('SELECT * FROM identities WHERE id = ?').get(retired.id),
          dependents: dependentSnapshot(database, retired.id),
        };
      });

      const listedBefore = await runCli(sandbox, ['identity', 'list', '--json']);
      expectJsonSuccess(listedBefore, { identities: [] });
      const missingBeforeReuse = await runCli(sandbox, ['identity', 'show', 'REUSABLE', '--json']);
      expect(missingBeforeReuse.status).toBe(3);
      expectError(missingBeforeReuse, 'NAME_NOT_FOUND');
      const replacementResult = await runCli(sandbox, ['identity', 'create', 'REUSABLE', '--json']);
      const replacement = documentIdentity(replacementResult);
      expect(replacement).toMatchObject({
        name: 'REUSABLE',
        canonicalName: retired.canonicalName,
        lifetime: 'saved',
      });
      expect(replacement.id).not.toBe(retired.id);
      expect(parseWholeStdout(replacementResult)).toEqual({
        identity: replacement,
        created: true,
      });
      expect(
        withDatabase(sandbox.database, (database) => ({
          old: database.prepare('SELECT * FROM identities WHERE id = ?').get(retired.id),
          oldDependents: dependentSnapshot(database, retired.id),
          newDependents: dependentSnapshot(database, replacement.id),
        }))
      ).toEqual({
        old: before.identity,
        oldDependents: before.dependents,
        newDependents: { role: undefined, preamble: undefined, binding: undefined },
      });
      expectJsonSuccess(await runCli(sandbox, ['identity', 'list', '--json']), {
        identities: [replacement],
      });
    });
  });

  it('uses canonical SQLite BINARY order for saved and temporary rows', async () => {
    await withSandbox(async (sandbox) => {
      const names = ['Zulu', 'Apple', 'Ä', 'zebra'];
      const identities = new Map<string, Identity>();
      for (const name of names) {
        const result = await runCli(sandbox, ['identity', 'create', name, '--json']);
        const identity = documentIdentity(result);
        identities.set(identity.canonicalName, identity);
      }

      const temporary = {
        id: '33333333-3333-4333-8333-333333333333',
        name: 'Temporary Row',
        canonicalName: 'temporary row',
        lifetime: 'temporary' as const,
      };
      withDatabase(sandbox.database, (database) => {
        seedIdentity(database, temporary);
      });
      const listed = await runCli(sandbox, ['identity', 'list', '--json']);
      expectJsonSuccess(listed, {
        identities: [
          identities.get('apple'),
          temporary,
          identities.get('zebra'),
          identities.get('zulu'),
          identities.get('ä'),
        ],
      });
    });
  });

  it('does not invoke tmux, read malformed unrelated config, or leak extra fields', async () => {
    await withSandbox(async (sandbox) => {
      const globalConfig = '{ malformed global config';
      const localConfig = '{ malformed local config';
      mkdirSync(sandbox.globalDir, { recursive: true });
      writeFileSync(sandbox.globalConfig, globalConfig);
      writeFileSync(sandbox.localConfig, localConfig);
      const logPath = await calibrateTmuxTripwire(sandbox);
      const calibratedLog = readFileSync(logPath, 'utf8');

      const created = await runCli(sandbox, ['identity', 'create', 'No Tmux', '--json']);
      expect(created.status).toBe(0);
      expect(readFileSync(logPath, 'utf8')).toBe(calibratedLog);
      const identity = documentIdentity(created);
      const before = withDatabase(sandbox.database, (database) => {
        seedDependents(database, identity.id, 'temporary');
        return dependentSnapshot(database, identity.id);
      });
      expectJsonSuccess(await runCli(sandbox, ['identity', 'show', 'no tmux', '--json']), {
        identity,
      });
      expect(readFileSync(logPath, 'utf8')).toBe(calibratedLog);
      expectJsonSuccess(await runCli(sandbox, ['identity', 'list', '--json']), {
        identities: [identity],
      });
      expect(readFileSync(logPath, 'utf8')).toBe(calibratedLog);
      expect(
        withDatabase(sandbox.database, (database) => dependentSnapshot(database, identity.id))
      ).toEqual(before);
      expect(readFileSync(sandbox.globalConfig, 'utf8')).toBe(globalConfig);
      expect(readFileSync(sandbox.localConfig, 'utf8')).toBe(localConfig);
      expect(Object.keys(parseWholeStdout(created)).sort()).toEqual(['created', 'identity']);
      expect(
        Object.keys((parseWholeStdout(created) as { identity: object }).identity).sort()
      ).toEqual(['canonicalName', 'id', 'lifetime', 'name']);
    });
  });

  it('honors a sandbox-contained explicit path override across directories', async () => {
    await withSandbox(async (sandbox) => {
      const override = path.join(sandbox.root, "custom root's files");
      const selected: Sandbox = {
        ...sandbox,
        cli: {
          executable: '/usr/bin/env',
          args: [`TMUX_TEAM_HOME=${override}`, sandbox.cli.executable, ...sandbox.cli.args],
        },
      };
      const nested = path.join(sandbox.cwd, 'override', 'child');
      mkdirSync(nested, { recursive: true });
      const child = { ...selected, cwd: nested };
      const created = documentIdentity(
        await runCli(selected, ['identity', 'create', 'Override', '--json'])
      );
      expect(existsSync(path.join(override, 'tmux-team.db'))).toBe(true);
      expect(existsSync(sandbox.database)).toBe(false);
      expectJsonSuccess(await runCli(child, ['identity', 'show', 'override', '--json']), {
        identity: created,
      });
      expectJsonSuccess(await runCli(child, ['identity', 'list', '--json']), {
        identities: [created],
      });
    });
  });

  it('keeps semantic validation and missing-name lookup bounded and distinct', async () => {
    await withSandbox(async (sandbox) => {
      for (const operation of ['create', 'show']) {
        for (const name of ['', ' \uFEFF ', 'bad\u007f', '%12', '10.3']) {
          const result = await runCli(sandbox, ['identity', operation, name, '--json']);
          expect(result.status, `${operation} ${JSON.stringify(name)}`).toBe(1);
          expectError(result, 'INVALID_NAME');
        }
      }
      expectJsonSuccess(await runCli(sandbox, ['identity', 'list', '--json']), { identities: [] });

      const missing = await runCli(sandbox, ['identity', 'show', 'Missing', '--json']);
      expect(missing.status).toBe(3);
      expect(missing.stderr).toBe('');
      expect(parseWholeStdout(missing)).toEqual({
        error: { code: 'NAME_NOT_FOUND', message: "Identity 'Missing' was not found." },
      });
    });
  });

  it('rejects grammar errors before opening storage', async () => {
    await withSandbox(async (sandbox) => {
      const syntax = await runCli(sandbox, ['identity', 'create', 'Alice', 'extra', '--json']);
      expect(syntax.status).toBe(1);
      expect(syntax.stderr).toBe('');
      expectError(syntax, 'USAGE_ERROR');
      expect(existsSync(sandbox.database)).toBe(false);
    });
  });

  it('maps corrupt and blocked storage to a bounded generic error', async () => {
    await withSandbox(async (sandbox) => {
      mkdirSync(sandbox.globalDir, { recursive: true });
      writeFileSync(sandbox.database, 'not a sqlite database');
      const corrupt = await runCli(sandbox, ['identity', 'list', '--json']);
      expect(corrupt.status).toBe(1);
      expectError(corrupt, 'IDENTITY_ERROR', 'Could not complete the identity operation.');
      expect(readFileSync(sandbox.database, 'utf8')).toBe('not a sqlite database');
    });

    await withSandbox(async (sandbox) => {
      mkdirSync(sandbox.xdgConfigHome, { recursive: true });
      writeFileSync(sandbox.globalDir, 'directory blocker');
      const blocked = await runCli(sandbox, ['identity', 'list', '--json']);
      expect(blocked.status).toBe(1);
      expectError(blocked, 'IDENTITY_ERROR', 'Could not complete the identity operation.');
      expect(readFileSync(sandbox.globalDir, 'utf8')).toBe('directory blocker');
    });
  });

  it('keeps human output explicit for empty, create, show, and list cases', async () => {
    await withSandbox(async (sandbox) => {
      delete sandbox.env.TMT_HINTS;
      const emptyHuman = await runCli(sandbox, ['identity', 'list']);
      expect(emptyHuman.status).toBe(0);
      expect(emptyHuman.stdout).toBe('No identities found.\nhint: tmt name <name>\n');
      expect(emptyHuman.stderr).toBe('');

      const human = await runCli(sandbox, ['identity', 'create', 'Human']);
      expect(human.status).toBe(0);
      expect(human.stdout).toContain("Created saved identity 'Human'");
      expect(human.stdout).toContain('saved');
      expect(human.stderr).toBe(
        'hint: receive work for this saved identity with tmt x listen --identity <name>\n'
      );
      const repeatedHuman = await runCli(sandbox, ['identity', 'create', 'human']);
      expect(repeatedHuman.status).toBe(0);
      expect(repeatedHuman.stdout).toContain("Saved identity 'Human' already exists");
      expect(repeatedHuman.stderr).toBe('');
      const humanShow = await runCli(sandbox, ['identity', 'show', 'human']);
      expect(humanShow.status).toBe(0);
      const shown = documentIdentity(
        await runCli(sandbox, ['identity', 'show', 'human', '--json'])
      );
      expect(humanShow.stdout).toBe(
        `Human\n  lifetime   saved\n  canonical  human\n  id         ${shown.id}\n`
      );
      expect(humanShow.stderr).toBe('');
      const missingHuman = await runCli(sandbox, ['identity', 'show', 'missing']);
      expect(missingHuman.status).toBe(3);
      expect(missingHuman.stdout).toBe('');
      expect(missingHuman.stderr).toBe("error: Identity 'missing' was not found\n");
      const humanList = await runCli(sandbox, ['identity', 'list']);
      expect(humanList.status).toBe(0);
      expect(humanList.stdout).toBe(`SAVED 1\n  Human  ${shown.id.slice(0, 8)}\n`);
      expect(humanList.stderr).toBe('');
    });
  });

  it('aligns Unicode and variable-length names in the human identity table', async () => {
    await withSandbox(async (sandbox) => {
      // Fullwidth and decomposed fixture names must retain their display bytes.
      const names = ['A', 'ＡＢ', 'Longest identity name', 'e\u0301'];
      const identities = new Map<string, Identity>();
      for (const name of names) {
        identities.set(
          name,
          documentIdentity(await runCli(sandbox, ['identity', 'create', name, '--json']))
        );
      }

      const listed = await runCli(sandbox, ['identity', 'list']);
      expect(listed.status).toBe(0);
      expect(listed.stderr).toBe('');
      expect(listed.stdout).toBe(
        `SAVED 4\n` +
          `  A                      ${identities.get('A')?.id.slice(0, 8)}\n` +
          `  ＡＢ                   ${identities.get('ＡＢ')?.id.slice(0, 8)}\n` +
          `  Longest identity name  ${identities.get('Longest identity name')?.id.slice(0, 8)}\n` +
          `  é                      ${identities.get('e\u0301')?.id.slice(0, 8)}\n`
      );
      const structured = await runCli(sandbox, ['identity', 'list', '--json']);
      expect(structured.status).toBe(0);
      expect(structured.stderr).toBe('');
      expect(parseWholeStdout(structured)).toEqual({
        identities: names.map((name, index) => ({
          id: identities.get(name)!.id,
          name,
          canonicalName: ['a', 'ab', 'longest identity name', 'é'][index],
          lifetime: 'saved',
        })),
      });
    });
  });

  it('sets, searches, lists, gets, removes, and retires exact identity metadata', async () => {
    await withSandbox(async (sandbox) => {
      const alice = documentIdentity(
        await runCli(sandbox, ['identity', 'create', 'Alice', '--json'])
      );
      const bob = documentIdentity(await runCli(sandbox, ['identity', 'create', 'Bob', '--json']));

      expectJsonSuccess(
        await runCli(sandbox, [
          'identity',
          'meta',
          'set',
          '--identity',
          'alice',
          'project',
          'tmt=alpha',
          '--json',
        ]),
        { identityId: alice.id, key: 'project', value: 'tmt=alpha', changed: true }
      );
      expectJsonSuccess(
        await runCli(sandbox, [
          'identity',
          'meta',
          'set',
          '--identity',
          'alice',
          'project',
          'tmt=alpha',
          '--json',
        ]),
        { identityId: alice.id, key: 'project', value: 'tmt=alpha', changed: false }
      );
      for (const [key, value] of [
        ['department', 'engineering'],
        ['capability.review', 'true'],
      ]) {
        expect(
          (await runCli(sandbox, ['identity', 'meta', 'set', '--identity', 'alice', key, value]))
            .status
        ).toBe(0);
      }
      expect(
        (
          await runCli(sandbox, [
            'identity',
            'meta',
            'set',
            '--identity',
            'bob',
            'project',
            'other',
          ])
        ).status
      ).toBe(0);

      const exact = await runCli(sandbox, [
        'identity',
        'meta',
        'get',
        '--identity',
        'alice',
        'project',
      ]);
      expect(exact).toMatchObject({ status: 0, stdout: 'tmt=alpha\n', stderr: '' });
      expectJsonSuccess(
        await runCli(sandbox, ['identity', 'meta', 'list', '--identity', 'alice', '--json']),
        {
          identityId: alice.id,
          metadata: {
            'capability.review': 'true',
            department: 'engineering',
            project: 'tmt=alpha',
          },
        }
      );
      expectJsonSuccess(
        await runCli(sandbox, [
          'identity',
          'list',
          '--where',
          'project=tmt=alpha',
          '--where',
          'department=engineering',
          '--has',
          'capability.review',
          '--json',
        ]),
        { identities: [alice] }
      );
      expectJsonSuccess(await runCli(sandbox, ['identity', 'list', '--has', 'missing', '--json']), {
        identities: [],
      });

      expectJsonSuccess(
        await runCli(sandbox, [
          'identity',
          'meta',
          'rm',
          '--identity',
          'alice',
          'project',
          '--json',
        ]),
        { identityId: alice.id, key: 'project', removed: true }
      );
      expectJsonSuccess(
        await runCli(sandbox, [
          'identity',
          'meta',
          'rm',
          '--identity',
          'alice',
          'project',
          '--json',
        ]),
        { identityId: alice.id, key: 'project', removed: false }
      );
      expectError(
        await runCli(sandbox, [
          'identity',
          'meta',
          'get',
          '--identity',
          'alice',
          'project',
          '--json',
        ]),
        'METADATA_KEY_NOT_FOUND',
        'The metadata key was not found.'
      );

      const removed = await runCli(sandbox, ['rm', 'Alice', '--force', '--json']);
      expect(removed.status).toBe(0);
      expect(
        withDatabase(sandbox.database, (database) =>
          database
            .prepare('SELECT COUNT(*) AS count FROM identity_metadata WHERE identity_id = ?')
            .get(alice.id)
        )
      ).toEqual({ count: 0 });
      expectJsonSuccess(
        await runCli(sandbox, ['identity', 'list', '--has', 'department', '--json']),
        {
          identities: [],
        }
      );
      expectJsonSuccess(await runCli(sandbox, ['identity', 'list', '--json']), {
        identities: [bob],
      });
    });
  });

  it('rejects invalid metadata and requires an explicit or verified caller identity', async () => {
    await withSandbox(async (sandbox) => {
      const missingIdentity = await runCli(sandbox, ['identity', 'meta', 'list', '--json']);
      expect(missingIdentity.status).toBe(1);
      expectError(
        missingIdentity,
        'IDENTITY_REQUIRED',
        'An identity is required; use --identity or run from a verified bound pane.'
      );
      expect(existsSync(sandbox.database)).toBe(false);

      const alice = documentIdentity(
        await runCli(sandbox, ['identity', 'create', 'Alice', '--json'])
      );
      expect(
        (
          await runCli(sandbox, [
            'identity',
            'meta',
            'set',
            '--identity',
            'alice',
            'preserved',
            'original',
            '--json',
          ])
        ).status
      ).toBe(0);
      for (const [key, value] of [
        ['Upper', 'value'],
        ['preserved', 'line\nfeed'],
        ['preserved', ''],
        ['preserved', 'é'.repeat(512) + 'a'],
      ]) {
        const result = await runCli(sandbox, [
          'identity',
          'meta',
          'set',
          '--identity',
          'alice',
          key,
          value,
          '--json',
        ]);
        expect(result.status).toBe(1);
        expectError(result, 'IDENTITY_METADATA_INVALID');
        expectJsonSuccess(
          await runCli(sandbox, [
            'identity',
            'meta',
            'get',
            '--identity',
            'alice',
            'preserved',
            '--json',
          ]),
          {
            identityId: alice.id,
            key: 'preserved',
            value: 'original',
          }
        );
      }
      const malformedFilter = await runCli(sandbox, [
        'identity',
        'list',
        '--where',
        'project',
        '--json',
      ]);
      expect(malformedFilter.status).toBe(1);
      const malformedDocument = expectError(malformedFilter, 'USAGE_ERROR');
      expect((malformedDocument.error as { message: string }).message).toContain('KEY=VALUE');
    });
  });

  it('rejects an unsupported future migration without mutating history or rows', async () => {
    await withSandbox(async (sandbox) => {
      await initializeSchema(sandbox);
      const created = documentIdentity(
        await runCli(sandbox, ['identity', 'create', 'History Fixture', '--json'])
      );
      const before = withDatabase(sandbox.database, (database) => ({
        history: database
          .prepare('SELECT version, name, applied_at FROM _migrations ORDER BY version')
          .all(),
        identities: database.prepare('SELECT * FROM identities ORDER BY id').all(),
      }));
      withDatabase(sandbox.database, (database) => {
        database
          .prepare('INSERT INTO _migrations (version, name, applied_at) VALUES (?, ?, ?)')
          .run(
            EXPECTED_NATIVE_SCHEMA_VERSION + 1,
            'unsupported future migration',
            '2026-01-07T00:00:00.000Z'
          );
      });
      const result = await runCli(sandbox, ['identity', 'show', created.canonicalName, '--json']);
      expect(result.status).toBe(1);
      expectError(result, 'IDENTITY_ERROR', 'Could not complete the identity operation.');
      expect(
        withDatabase(sandbox.database, (database) => ({
          history: database
            .prepare('SELECT version, name, applied_at FROM _migrations ORDER BY version')
            .all(),
          identities: database.prepare('SELECT * FROM identities ORDER BY id').all(),
        }))
      ).toEqual({
        history: [
          ...before.history,
          {
            version: EXPECTED_NATIVE_SCHEMA_VERSION + 1,
            name: 'unsupported future migration',
            applied_at: '2026-01-07T00:00:00.000Z',
          },
        ],
        identities: before.identities,
      });
    });
  });
});
