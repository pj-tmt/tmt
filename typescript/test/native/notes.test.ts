import Database from 'better-sqlite3';
import {
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  realpathSync,
  rmSync,
  statSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import {
  expectError,
  parseWholeStdout,
  runCli,
  type Sandbox,
  withSandbox,
} from '../support/cli-process.js';

type CreatedIdentity = {
  readonly identity: {
    readonly id: string;
    readonly name: string;
    readonly canonicalName: string;
    readonly lifetime: 'saved' | 'temporary';
  };
  readonly created: boolean;
};

async function createIdentity(
  sandbox: Sandbox,
  name: string
): Promise<CreatedIdentity['identity']> {
  const result = await runCli(sandbox, ['identity', 'create', name, '--json']);
  expect(result.status).toBe(0);
  return (parseWholeStdout(result) as unknown as CreatedIdentity).identity;
}

function openDatabase<T>(sandbox: Sandbox, callback: (database: Database.Database) => T): T {
  const database = new Database(sandbox.database);
  try {
    return callback(database);
  } finally {
    database.close();
  }
}

describe('saved identity notes path', () => {
  it('creates one private absolute Markdown file and preserves exact existing bytes and inode', async () => {
    await withSandbox(async (sandbox) => {
      const identity = await createIdentity(sandbox, 'Research & QA / 東京');
      const expected = path.join(sandbox.globalDir, 'notes', identity.id, 'notes.md');

      const plain = await runCli(sandbox, ['notes', 'path', '--identity', 'RESEARCH & QA / 東京']);
      expect(plain).toEqual({ status: 0, signal: null, stdout: `${expected}\n`, stderr: '' });
      expect(path.isAbsolute(plain.stdout.trim())).toBe(true);
      expect(readFileSync(expected)).toEqual(Buffer.alloc(0));
      expect(statSync(path.dirname(expected)).mode & 0o777).toBe(0o700);
      expect(statSync(path.dirname(path.dirname(expected))).mode & 0o777).toBe(0o700);
      expect(statSync(expected).mode & 0o777).toBe(0o600);

      const exact = Buffer.from([0x23, 0x20, 0x4e, 0x6f, 0x74, 0x65, 0x73, 0x0a, 0x00, 0xff]);
      writeFileSync(expected, exact);
      const inode = statSync(expected).ino;
      const repeated = await runCli(sandbox, [
        'notes',
        'path',
        '--identity',
        identity.name,
        '--json',
      ]);
      expect(repeated.status).toBe(0);
      expect(repeated.stderr).toBe('');
      expect(parseWholeStdout(repeated)).toEqual({
        identityId: identity.id,
        path: expected,
        created: false,
      });
      expect(readFileSync(expected)).toEqual(exact);
      expect(statSync(expected).ino).toBe(inode);

      expect(
        (await runCli(sandbox, ['role', 'set', 'analysis role', '--identity', identity.name]))
          .status
      ).toBe(0);
      const nested = path.join(sandbox.cwd, 'nested', 'workspace');
      mkdirSync(nested, { recursive: true });
      const fromOtherDirectory = await runCli({ ...sandbox, cwd: nested }, [
        'notes',
        'path',
        '--identity',
        identity.name,
        '--json',
      ]);
      expect(parseWholeStdout(fromOtherDirectory)).toEqual({
        identityId: identity.id,
        path: expected,
        created: false,
      });
      expect(readFileSync(expected)).toEqual(exact);
    });
  });

  it('has exactly one creator under concurrent first access', async () => {
    await withSandbox(async (sandbox) => {
      const identity = await createIdentity(sandbox, 'Concurrent');
      const results = await Promise.all(
        Array.from({ length: 8 }, () =>
          runCli(sandbox, ['notes', 'path', '--identity', identity.name, '--json'])
        )
      );
      const documents = results.map((result) => {
        expect(result.status).toBe(0);
        expect(result.stderr).toBe('');
        return parseWholeStdout(result) as {
          identityId: string;
          path: string;
          created: boolean;
        };
      });
      expect(documents.filter((document) => document.created)).toHaveLength(1);
      const expected = path.join(sandbox.globalDir, 'notes', identity.id, 'notes.md');
      expect(new Set(documents.map((document) => document.path))).toEqual(new Set([expected]));
      expect(lstatSync(expected).isFile()).toBe(true);
      expect(readFileSync(expected)).toEqual(Buffer.alloc(0));

      const sentinel = Buffer.from('# concurrent sentinel\n');
      writeFileSync(expected, sentinel);
      const inode = statSync(expected).ino;
      const repeats = await Promise.all(
        Array.from({ length: 8 }, () =>
          runCli(sandbox, ['notes', 'path', '--identity', identity.name, '--json'])
        )
      );
      for (const result of repeats) {
        expect(result.status).toBe(0);
        expect(result.stderr).toBe('');
        expect(parseWholeStdout(result)).toEqual({
          identityId: identity.id,
          path: expected,
          created: false,
        });
      }
      expect(statSync(expected).ino).toBe(inode);
      expect(readFileSync(expected)).toEqual(sentinel);
    });
  });

  it('rejects temporary, unknown, retired, and corrupt stored identities without a notebook', async () => {
    await withSandbox(async (sandbox) => {
      await createIdentity(sandbox, 'Schema owner');
      const temporaryId = '11111111-1111-4111-8111-111111111111';
      const retiredId = '22222222-2222-4222-8222-222222222222';
      openDatabase(sandbox, (database) => {
        const insert = database.prepare(
          `INSERT INTO identities
             (id, name, canonical_name, created_at, updated_at, lifetime, retired_at_ms)
           VALUES (?, ?, ?, ?, ?, ?, ?)`
        );
        insert.run(
          temporaryId,
          'Temporary',
          'temporary',
          '2026-01-01T00:00:00Z',
          '2026-01-01T00:00:00Z',
          'temporary',
          null
        );
        insert.run(
          retiredId,
          'Retired',
          'retired',
          '2026-01-01T00:00:00Z',
          '2026-01-01T00:00:00Z',
          'saved',
          1
        );
        insert.run(
          '../escape',
          'Corrupt',
          'corrupt',
          '2026-01-01T00:00:00Z',
          '2026-01-01T00:00:00Z',
          'saved',
          null
        );
      });

      const temporary = await runCli(sandbox, [
        'notes',
        'path',
        '--identity',
        'Temporary',
        '--json',
      ]);
      expect(temporary.status).toBe(1);
      expectError(temporary, 'NOTES_SAVED_IDENTITY_REQUIRED');
      expect(existsSync(path.join(sandbox.globalDir, 'notes', temporaryId))).toBe(false);

      const promoted = await runCli(sandbox, ['identity', 'create', 'Temporary', '--json']);
      expect(promoted.status).toBe(0);
      expect(parseWholeStdout(promoted)).toMatchObject({
        identity: { id: temporaryId, lifetime: 'saved' },
        created: false,
      });
      const promotedNotes = await runCli(sandbox, [
        'notes',
        'path',
        '--identity',
        'Temporary',
        '--json',
      ]);
      expect(parseWholeStdout(promotedNotes)).toEqual({
        identityId: temporaryId,
        path: path.join(sandbox.globalDir, 'notes', temporaryId, 'notes.md'),
        created: true,
      });

      for (const name of ['Missing', 'Retired']) {
        const missing = await runCli(sandbox, ['notes', 'path', '--identity', name, '--json']);
        expect(missing.status).toBe(3);
        expectError(missing, 'NAME_NOT_FOUND');
      }
      expect(existsSync(path.join(sandbox.globalDir, 'notes', retiredId))).toBe(false);

      const corrupt = await runCli(sandbox, ['notes', 'path', '--identity', 'Corrupt', '--json']);
      expect(corrupt.status).toBe(1);
      expectError(corrupt, 'NOTES_IO_ERROR');
      expect(existsSync(path.join(sandbox.globalDir, 'escape'))).toBe(false);
    });
  });

  it('requires an explicit identity outside tmux before creating storage', async () => {
    await withSandbox(async (sandbox) => {
      const result = await runCli(sandbox, ['notes', 'path', '--json']);
      expect(result.status).toBe(1);
      expectError(result, 'IDENTITY_REQUIRED');
      expect(existsSync(sandbox.globalDir)).toBe(false);
    });
  });

  it('retains retired notes and gives a same-name replacement a new path', async () => {
    await withSandbox(async (sandbox) => {
      const original = await createIdentity(sandbox, 'Reusable');
      const first = parseWholeStdout(
        await runCli(sandbox, ['notes', 'path', '--identity', 'Reusable', '--json'])
      ) as { path: string };
      writeFileSync(first.path, '# retained\n');
      expect((await runCli(sandbox, ['rm', 'Reusable', '--force', '--json'])).status).toBe(0);

      const replacement = await createIdentity(sandbox, 'REUSABLE');
      expect(replacement.id).not.toBe(original.id);
      const secondResult = await runCli(sandbox, [
        'notes',
        'path',
        '--identity',
        'reusable',
        '--json',
      ]);
      const second = parseWholeStdout(secondResult) as { path: string; created: boolean };
      expect(second.created).toBe(true);
      expect(second.path).not.toBe(first.path);
      expect(readFileSync(first.path, 'utf8')).toBe('# retained\n');
      expect(readFileSync(second.path)).toEqual(Buffer.alloc(0));
    });
  });

  it('rejects symlinked and nonregular notebook targets without touching sentinels', async () => {
    await withSandbox(async (sandbox) => {
      const identity = await createIdentity(sandbox, 'Safety');
      const notesRoot = path.join(sandbox.globalDir, 'notes');
      const outside = path.join(sandbox.root, 'outside');
      mkdirSync(outside);
      writeFileSync(path.join(outside, 'sentinel'), 'untouched');
      symlinkSync(outside, notesRoot);
      const linked = await runCli(sandbox, [
        'notes',
        'path',
        '--identity',
        identity.name,
        '--json',
      ]);
      expect(linked.status).toBe(1);
      expectError(linked, 'NOTES_IO_ERROR');
      expect(readFileSync(path.join(outside, 'sentinel'), 'utf8')).toBe('untouched');

      rmSync(notesRoot);
      const identityDir = path.join(notesRoot, identity.id);
      mkdirSync(notesRoot);
      symlinkSync(outside, identityDir);
      const linkedIdentity = await runCli(sandbox, [
        'notes',
        'path',
        '--identity',
        identity.name,
        '--json',
      ]);
      expect(linkedIdentity.status).toBe(1);
      expectError(linkedIdentity, 'NOTES_IO_ERROR');
      expect(readFileSync(path.join(outside, 'sentinel'), 'utf8')).toBe('untouched');
      expect(existsSync(path.join(outside, 'notes.md'))).toBe(false);

      rmSync(identityDir);
      mkdirSync(identityDir);
      const target = path.join(identityDir, 'notes.md');
      const sentinel = path.join(outside, 'sentinel');
      symlinkSync(sentinel, target);
      const linkedFile = await runCli(sandbox, [
        'notes',
        'path',
        '--identity',
        identity.name,
        '--json',
      ]);
      expect(linkedFile.status).toBe(1);
      expectError(linkedFile, 'NOTES_IO_ERROR');
      expect(lstatSync(target).isSymbolicLink()).toBe(true);
      expect(readFileSync(sentinel, 'utf8')).toBe('untouched');

      rmSync(target);
      const missing = path.join(outside, 'missing.md');
      symlinkSync(missing, target);
      const danglingFile = await runCli(sandbox, [
        'notes',
        'path',
        '--identity',
        identity.name,
        '--json',
      ]);
      expect(danglingFile.status).toBe(1);
      expectError(danglingFile, 'NOTES_IO_ERROR');
      expect(lstatSync(target).isSymbolicLink()).toBe(true);
      expect(existsSync(missing)).toBe(false);

      rmSync(target);
      mkdirSync(target);
      const nonregular = await runCli(sandbox, [
        'notes',
        'path',
        '--identity',
        identity.name,
        '--json',
      ]);
      expect(nonregular.status).toBe(1);
      expectError(nonregular, 'NOTES_IO_ERROR');
      expect(lstatSync(target).isDirectory()).toBe(true);
    });
  });

  it('honors a relative explicit config root and still prints an absolute path', async () => {
    await withSandbox(async (sandbox) => {
      const selected: Sandbox = {
        ...sandbox,
        cli: {
          executable: '/usr/bin/env',
          args: ['TMT_HOME=relative-state', sandbox.cli.executable, ...sandbox.cli.args],
        },
      };
      const identity = await createIdentity(selected, 'Relative');
      const result = await runCli(selected, [
        'notes',
        'path',
        '--identity',
        identity.name,
        '--json',
      ]);
      expect(result.status).toBe(0);
      expect(parseWholeStdout(result)).toEqual({
        identityId: identity.id,
        path: path.join(
          realpathSync(sandbox.cwd),
          'relative-state',
          'notes',
          identity.id,
          'notes.md'
        ),
        created: true,
      });
    });
  });
});
