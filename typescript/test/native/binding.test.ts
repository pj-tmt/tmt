import Database from 'better-sqlite3';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { calibrateTmuxTripwire } from './tmux-tripwire.js';
import { expectError, parseWholeStdout, runCli, withSandbox } from '../support/cli-process.js';

type Identity = {
  readonly id: string;
  readonly name: string;
  readonly canonicalName: string;
  readonly lifetime: 'temporary' | 'saved';
};

function documentIdentity(result: Awaited<ReturnType<typeof runCli>>): Identity {
  expect(result.status).toBe(0);
  expect(result.stderr).toBe('');
  const document = parseWholeStdout(result) as { identity?: unknown };
  expect(document.identity).toMatchObject({
    id: expect.any(String),
    name: expect.any(String),
    canonicalName: expect.any(String),
    lifetime: expect.stringMatching(/^(temporary|saved)$/),
  });
  expect(Object.keys(document.identity as object).sort()).toEqual([
    'canonicalName',
    'id',
    'lifetime',
    'name',
  ]);
  return document.identity as Identity;
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

function seedExchange(database: Database.Database, identityId: string): void {
  database
    .prepare(
      `
    INSERT INTO request_attempts (
      attempt_id, request_id, identity_id, server_id, socket_path, server_pid,
      server_start_time, pane_id, pane_pid, wait_active, status,
      inject_preamble, cadence_reserved, prepared_at_ms, expires_at_ms,
      originator_kind, originator_identity_id, attention_revision
    ) VALUES (
      'attempt-retained', 'request-retained', ?, 'server-retained',
      '/tmp/retained.sock', 101, 'start-retained', '%12', 202, 0, 'sent',
      0, 0, 1700000000000, 1700000010000, 'explicit', ?, 1
    )
  `
    )
    .run(identityId, identityId);
  database
    .prepare(
      `
      INSERT INTO request_responses (
      request_id, attempt_id, server_id, socket_path, server_pid,
      server_start_time, pane_id, pane_pid, body, body_bytes, submitted_at_ms
      ) VALUES (
      'request-retained', 'attempt-retained', 'server-retained', '/tmp/retained.sock',
      101, 'start-retained', '%12', 202, 'retained exchange', 17, 1700000000200
      )
    `
    )
    .run();
}

describe('native identity binding process contract', () => {
  it('runs pane preflight before malformed config or invalid names and leaves no effects', async () => {
    await withSandbox(async (sandbox) => {
      mkdirSync(sandbox.globalDir, { recursive: true });
      const globalConfig = '{ malformed global config';
      const localConfig = '{ malformed local config';
      writeFileSync(sandbox.globalConfig, globalConfig);
      writeFileSync(sandbox.localConfig, localConfig);
      const logPath = await calibrateTmuxTripwire(sandbox);
      const calibratedLog = readFileSync(logPath, 'utf8');

      for (const args of [
        ['name', 'bad\u007f', '--json'],
        ['this', 'bad\u007f', '--json'],
        ['add', '%99', 'bad\u007f', '--json'],
        ['whoami', '--json'],
        ['unbind', '--json'],
      ]) {
        const result = await runCli(sandbox, args);
        expect(result.status, args.join(' ')).toBe(3);
        expectError(result, 'PANE_NOT_FOUND');
        expect(result.stderr).toBe('');
        expect(existsSync(sandbox.database)).toBe(false);
        expect(readFileSync(sandbox.globalConfig, 'utf8')).toBe(globalConfig);
        expect(readFileSync(sandbox.localConfig, 'utf8')).toBe(localConfig);
      }
      expect(readFileSync(logPath, 'utf8')).not.toBe(calibratedLog);
    });
  });

  it('reports the legacy add order before probing or opening storage', async () => {
    await withSandbox(async (sandbox) => {
      const logPath = await calibrateTmuxTripwire(sandbox);
      const before = readFileSync(logPath, 'utf8');
      const result = await runCli(sandbox, ['add', 'worker', '%99', '--json']);
      expect(result.status).toBe(1);
      expectError(result, 'LEGACY_ADD_ORDER');
      expect(parseWholeStdout(result)).toEqual({
        error: {
          code: 'LEGACY_ADD_ORDER',
          message: 'The v4 add argument order is no longer supported.',
          suggestion: 'Use: tmt add %99 worker',
        },
      });
      expect(result.stderr).toBe('');
      expect(readFileSync(logPath, 'utf8')).toBe(before);
      expect(existsSync(sandbox.database)).toBe(false);
    });
  });

  it('lists an offline identity globally and by name without exposing binding evidence', async () => {
    await withSandbox(async (sandbox) => {
      const logPath = await calibrateTmuxTripwire(sandbox);
      const calibratedLog = readFileSync(logPath, 'utf8');
      const identity = documentIdentity(
        await runCli(sandbox, ['identity', 'create', 'Offline Agent', '--json'])
      );

      const global = await runCli(sandbox, ['ls', '--json']);
      expect(global.status).toBe(0);
      expect(global.stderr).toBe('');
      expect(parseWholeStdout(global)).toEqual({
        identities: [
          {
            ...identity,
            presence: 'offline',
            pane: null,
            command: '',
            // Additive: an offline identity with no remembered session has no address.
            address: null,
            driver: null,
            session: {
              activity: { state: 'unknown', sinceMs: null, lastActivityMs: null, providers: {} },
            },
          },
        ],
      });
      const globalRow = (parseWholeStdout(global) as { identities: Array<Record<string, unknown>> })
        .identities[0];
      expect(Object.keys(globalRow).sort()).toEqual([
        'address',
        'canonicalName',
        'command',
        'driver',
        'id',
        'lifetime',
        'name',
        'pane',
        'presence',
        'session',
      ]);

      const named = await runCli(sandbox, ['list', 'offline agent', '--json']);
      expect(named.status).toBe(0);
      expect(named.stderr).toBe('');
      expect(parseWholeStdout(named)).toEqual({
        target: 'offline agent',
        identity,
        presence: 'offline',
        pane: null,
        address: null,
        driver: null,
      });
      expect(Object.keys(parseWholeStdout(named)).sort()).toEqual([
        'address',
        'driver',
        'identity',
        'pane',
        'presence',
        'target',
      ]);

      const missing = await runCli(sandbox, ['list', 'missing', '--json']);
      expect(missing.status).toBe(3);
      expectError(missing, 'NAME_NOT_FOUND');
      expect(readFileSync(logPath, 'utf8')).toBe(calibratedLog);

      const human = await runCli(sandbox, ['ls']);
      expect(human.status).toBe(0);
      expect(human.stderr).toBe('');
      // Offline with nothing to resume: folded into one line; --all expands it.
      expect(human.stdout).toBe('SAVED 1\n    offline: Offline Agent\n');
      const all = await runCli(sandbox, ['ls', '--all']);
      expect(all.stdout).toBe('SAVED 1\n  ○  Offline Agent  -\n');
      const details = await runCli(sandbox, ['ls', 'offline agent']);
      expect(details.stdout).toBe(
        'Offline Agent\n' +
          '  lifetime  saved\n' +
          '  state     ○ offline\n' +
          '  address   -\n' +
          '  pane      -\n' +
          '  target    -\n' +
          '  cwd       -\n' +
          '  command   -\n'
      );
    });
  });

  it('filters ls by lifetime and refuses --here outside tmux', async () => {
    await withSandbox(async (sandbox) => {
      await runCli(sandbox, ['identity', 'create', 'Keeper', '--json']);
      const saved = await runCli(sandbox, ['ls', '--saved', '--all']);
      expect(saved.stdout).toBe('SAVED 1\n  ○  Keeper  -\n');
      const temporary = await runCli(sandbox, ['ls', '--temp']);
      expect(temporary.status).toBe(0);
      expect(temporary.stdout).toBe('No identities found.\nhint: tmt name <name>\n');
      const here = await runCli(sandbox, ['ls', '--here']);
      expect(here.status).toBe(3);
      expect(here.stdout).toBe('');
      expect(here.stderr).toContain('needs a tmux pane');
      const json = await runCli(sandbox, ['ls', '--temp', '--json']);
      expect(parseWholeStdout(json)).toEqual({ identities: [] });
    });
  });

  it('aligns Unicode and variable-length names in the human ls list', async () => {
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

      const listed = await runCli(sandbox, ['ls', '--all']);
      expect(listed.status).toBe(0);
      expect(listed.stderr).toBe('');
      // Sorted by canonical name; widths are display columns, not bytes.
      expect(listed.stdout).toBe(
        'SAVED 4\n' +
          '  ○  A                      -\n' +
          '  ○  ＡＢ                   -\n' +
          '  ○  Longest identity name  -\n' +
          '  ○  e\u0301                      -\n'
      );
      const folded = await runCli(sandbox, ['ls']);
      expect(folded.stdout).toBe(
        'SAVED 4\n    offline: A · ＡＢ · Longest identity name · e\u0301\n'
      );
      const structured = await runCli(sandbox, ['ls', '--json']);
      expect(structured.status).toBe(0);
      expect(structured.stderr).toBe('');
      expect(parseWholeStdout(structured)).toEqual({
        identities: names.map((name, index) => ({
          id: identities.get(name)!.id,
          name,
          canonicalName: ['a', 'ab', 'longest identity name', 'é'][index],
          lifetime: 'saved',
          presence: 'offline',
          pane: null,
          command: '',
          address: null,
          driver: null,
          session: {
            activity: { state: 'unknown', sinceMs: null, lastActivityMs: null, providers: {} },
          },
        })),
      });
    });
  });

  it('requires rm confirmation and force-retirement preserves exchange rows', async () => {
    await withSandbox(async (sandbox) => {
      const identity = documentIdentity(
        await runCli(sandbox, ['identity', 'create', 'Retained Exchange', '--json'])
      );
      withDatabase(sandbox.database, (database) => seedExchange(database, identity.id));

      const before = withDatabase(sandbox.database, (database) => ({
        identity: database
          .prepare('SELECT id, retired_at_ms FROM identities WHERE id = ?')
          .get(identity.id),
        attempt: database
          .prepare(
            'SELECT identity_id, originator_identity_id, status FROM request_attempts WHERE attempt_id = ?'
          )
          .get('attempt-retained'),
        response: database
          .prepare('SELECT body, body_bytes FROM request_responses WHERE request_id = ?')
          .get('request-retained'),
      }));

      const confirmation = await runCli(sandbox, ['rm', 'retained exchange', '--json']);
      expect(confirmation.status).toBe(5);
      expectError(
        confirmation,
        'CONFIRMATION_REQUIRED',
        'Saved identity removal requires --force. Its role and preamble will be removed; exchanges are retained.'
      );
      expect(
        withDatabase(sandbox.database, (database) => ({
          identity: database
            .prepare('SELECT id, retired_at_ms FROM identities WHERE id = ?')
            .get(identity.id),
          attempt: database
            .prepare(
              'SELECT identity_id, originator_identity_id, status FROM request_attempts WHERE attempt_id = ?'
            )
            .get('attempt-retained'),
          response: database
            .prepare('SELECT body, body_bytes FROM request_responses WHERE request_id = ?')
            .get('request-retained'),
        }))
      ).toEqual(before);

      const removed = await runCli(sandbox, ['remove', 'retained exchange', '--force', '--json']);
      expect(removed.status).toBe(0);
      expect(removed.stderr).toBe('');
      expect(parseWholeStdout(removed)).toEqual({
        removed: true,
        identity,
      });
      const missing = await runCli(sandbox, ['identity', 'show', 'retained exchange', '--json']);
      expect(missing.status).toBe(3);
      expectError(missing, 'NAME_NOT_FOUND');

      expect(
        withDatabase(sandbox.database, (database) => ({
          identity: database
            .prepare('SELECT id, retired_at_ms FROM identities WHERE id = ?')
            .get(identity.id),
          attempt: database
            .prepare(
              'SELECT identity_id, originator_identity_id, status FROM request_attempts WHERE attempt_id = ?'
            )
            .get('attempt-retained'),
          response: database
            .prepare('SELECT body, body_bytes FROM request_responses WHERE request_id = ?')
            .get('request-retained'),
        }))
      ).toEqual({
        identity: { id: identity.id, retired_at_ms: expect.any(Number) },
        attempt: { identity_id: identity.id, originator_identity_id: identity.id, status: 'sent' },
        response: { body: 'retained exchange', body_bytes: 17 },
      });
    });
  });
});
