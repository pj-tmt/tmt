import Database from 'better-sqlite3';
import { execFileSync } from 'node:child_process';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import {
  expectError,
  parseWholeStdout,
  runCli,
  withSandbox,
  type Sandbox,
} from '../support/cli-process.js';
import { calibrateTmuxTripwire } from './tmux-tripwire.js';
import { EXPECTED_NATIVE_SCHEMA_VERSION } from './storage-fixture.js';

async function json(sandbox: Sandbox, args: string[]): Promise<unknown> {
  const result = await runCli(sandbox, [...args, '--json']);
  expect(result.status, result.stdout || result.stderr).toBe(0);
  expect(result.stderr).toBe('');
  return parseWholeStdout(result);
}

describe('native role and preamble process contracts', () => {
  it('prints the role lifecycle in human mode and retains the durable profile', async () => {
    await withSandbox(async (sandbox) => {
      await json(sandbox, ['identity', 'create', 'Alice']);

      const set = await runCli(sandbox, ['role', 'set', 'Review code', '--identity', 'Alice']);
      expect(set.status).toBe(0);
      expect(set.stdout).toBe("✓ Set role profile for 'Alice'\n");
      expect(set.stderr).toBe('');

      const shown = await runCli(sandbox, ['role', 'show', '--identity', 'Alice']);
      expect(shown.status).toBe(0);
      expect(shown.stdout).toBe("Identity 'Alice'\nReview code\n");
      expect(shown.stderr).toBe('');

      const cleared = await runCli(sandbox, ['role', 'clear', '--identity', 'Alice']);
      expect(cleared.status).toBe(0);
      expect(cleared.stdout).toBe("✓ Cleared role profile for 'Alice'\n");
      expect(cleared.stderr).toBe('');
      expect(await json(sandbox, ['role', 'show', '--identity', 'Alice'])).toMatchObject({
        identity: { name: 'Alice', lifetime: 'saved' },
        role: null,
      });
    });
  });

  it('reports human role file failures without changing the stored profile', async () => {
    await withSandbox(async (sandbox) => {
      const log = await calibrateTmuxTripwire(sandbox);
      await json(sandbox, ['identity', 'create', 'Alice']);
      const original = await json(sandbox, ['role', 'set', 'Review code', '--identity', 'Alice']);
      const result = await runCli(sandbox, [
        'role',
        'set',
        '--file',
        path.join(sandbox.root, 'missing-role.txt'),
        '--identity',
        'Alice',
      ]);
      expect(result.status).toBe(1);
      expect(result.stdout).toBe('');
      expect(result.stderr).toBe('error: Could not read a regular role file\n');
      expect(await json(sandbox, ['role', 'show', '--identity', 'Alice'])).toEqual(original);
      expect(readFileSync(log, 'utf8')).toBe('\n');
    });
  });

  it('prints the preamble show/set/clear lifecycle and preserves the paired JSON state', async () => {
    await withSandbox(async (sandbox) => {
      await json(sandbox, ['identity', 'create', 'Alice']);

      const set = await runCli(sandbox, ['preamble', 'set', 'Alice', 'Be helpful']);
      expect(set.status).toBe(0);
      expect(set.stdout).toBe('✓ Set preamble for Alice\n');
      expect(set.stderr).toBe('');

      const shown = await runCli(sandbox, ['preamble', 'show', 'Alice']);
      expect(shown.status).toBe(0);
      expect(shown.stdout).toBe('Preamble for Alice:\nBe helpful\n');
      expect(shown.stderr).toBe('');
      expect(await json(sandbox, ['preamble', 'show', 'Alice'])).toEqual({
        agent: 'Alice',
        preamble: 'Be helpful',
      });

      const cleared = await runCli(sandbox, ['preamble', 'clear', 'Alice']);
      expect(cleared.status).toBe(0);
      expect(cleared.stdout).toBe('✓ Cleared preamble for Alice\n');
      expect(cleared.stderr).toBe('');
      expect(await json(sandbox, ['preamble', 'show', 'Alice'])).toEqual({
        agent: 'Alice',
        preamble: null,
      });
    });
  });

  it('transports a large multibyte role as one complete JSON document', async () => {
    await withSandbox(async (sandbox) => {
      const log = await calibrateTmuxTripwire(sandbox);
      const created = (await json(sandbox, ['identity', 'create', 'LargeRole'])) as {
        identity: unknown;
      };
      // Non-English fixture bytes exercise multibyte JSON transport, not UI copy.
      const content = 'role-content-日本語-🚀-' + 'x'.repeat(48 * 1024);
      const file = path.join(sandbox.root, 'large-role.txt');
      writeFileSync(file, content);
      await json(sandbox, ['role', 'set', '--file', file, '--identity', 'LargeRole']);
      const result = await runCli(sandbox, ['role', 'show', '--identity', 'LargeRole', '--json']);
      expect(result.status).toBe(0);
      expect(result.stderr).toBe('');
      expect(Buffer.byteLength(result.stdout)).toBeGreaterThan(32 * 1024);
      expect(parseWholeStdout(result)).toEqual({
        identity: created.identity,
        role: { content, updatedAt: expect.any(String) },
      });
      expect(readFileSync(log, 'utf8')).toBe('\n');
    });
  });

  it('maps a future schema to one role error without mutating history or identities', async () => {
    await withSandbox(async (sandbox) => {
      await json(sandbox, ['identity', 'create', 'Preserved']);
      const writer = new Database(sandbox.database);
      let history: unknown[];
      let identities: unknown[];
      try {
        writer
          .prepare(
            "INSERT INTO _migrations VALUES (?, 'future migration', '2026-01-01T00:00:00.000Z')"
          )
          .run(EXPECTED_NATIVE_SCHEMA_VERSION + 1);
        history = writer.prepare('SELECT * FROM _migrations ORDER BY version').all();
        identities = writer.prepare('SELECT * FROM identities ORDER BY id').all();
      } finally {
        writer.close();
      }
      const result = await runCli(sandbox, ['role', 'show', '--identity', 'Preserved', '--json']);
      expect(result.status).toBe(1);
      expectError(result, 'ROLE_ERROR');
      const reader = new Database(sandbox.database, { readonly: true });
      try {
        expect(reader.prepare('SELECT * FROM _migrations ORDER BY version').all()).toEqual(history);
        expect(reader.prepare('SELECT * FROM identities ORDER BY id').all()).toEqual(identities);
      } finally {
        reader.close();
      }
    });
  });

  it('persists independent offline profiles across processes without config or tmux', async () => {
    await withSandbox(async (sandbox) => {
      const log = await calibrateTmuxTripwire(sandbox);
      await json(sandbox, ['identity', 'create', 'Alice']);
      await json(sandbox, ['identity', 'create', 'Bob']);
      mkdirSync(sandbox.globalDir, { recursive: true });
      writeFileSync(sandbox.globalConfig, '{ invalid');
      writeFileSync(sandbox.localConfig, '{ invalid');
      const original = '\ufeff  role\r\nline\r\tend  ';
      const normalized = '  role\nline\n\tend  ';
      const set = await json(sandbox, ['role', '--identity', 'aLiCe', 'set', original]);
      expect(set).toMatchObject({
        identity: { name: 'Alice', canonicalName: 'alice', lifetime: 'saved' },
        role: { content: normalized, updatedAt: expect.any(String) },
      });
      expect(await json(sandbox, ['role', 'show', '--identity', 'Alice'])).toEqual(set);
      expect(await json(sandbox, ['preamble', 'set', 'Bob', 'first', 'second'])).toEqual({
        agent: 'Bob',
        preamble: 'first second',
        status: 'set',
      });
      expect(await json(sandbox, ['preamble', 'set', 'Bob', 'beta'])).toEqual({
        agent: 'Bob',
        preamble: 'beta',
        status: 'set',
      });
      expect(await json(sandbox, ['preamble', 'set', 'Alice', '\ufeffprefix\r\n'])).toEqual({
        agent: 'Alice',
        preamble: 'prefix\n',
        status: 'set',
      });
      expect(await json(sandbox, ['preamble'])).toEqual({
        preambles: [
          { agent: 'Alice', preamble: 'prefix\n' },
          { agent: 'Bob', preamble: 'beta' },
        ],
      });
      expect(await json(sandbox, ['role', 'show', '--identity', 'Bob'])).toMatchObject({
        role: null,
      });
      expect(await json(sandbox, ['role', 'clear', '--identity', 'Alice'])).toMatchObject({
        role: null,
      });
      expect(await json(sandbox, ['role', 'clear', '--identity', 'Alice'])).toMatchObject({
        role: null,
      });
      expect(await json(sandbox, ['preamble', 'show', 'Alice'])).toEqual({
        agent: 'Alice',
        preamble: 'prefix\n',
      });
      expect(await json(sandbox, ['preamble', 'clear', 'Alice'])).toEqual({
        agent: 'Alice',
        status: 'cleared',
      });
      expect(await json(sandbox, ['preamble', 'clear', 'Alice'])).toEqual({
        agent: 'Alice',
        status: 'not_set',
      });
      expect(await json(sandbox, ['preamble', 'show', 'Alice'])).toEqual({
        agent: 'Alice',
        preamble: null,
      });
      expect(readFileSync(log, 'utf8')).toBe('\n');
    });
  });

  it('rejects unknown selectors and invalid text without creating or altering profiles', async () => {
    await withSandbox(async (sandbox) => {
      const log = await calibrateTmuxTripwire(sandbox);
      await json(sandbox, ['identity', 'create', 'Alice']);
      const initial = await json(sandbox, ['role', 'set', 'preserved', '--identity', 'Alice']);
      for (const args of [
        ['role', 'show', '--identity', 'bad\nname'],
        ['preamble', 'show', '%14'],
      ]) {
        const result = await runCli(sandbox, [...args, '--json']);
        expect(result.status).toBe(3);
        expectError(result, 'NAME_NOT_FOUND');
      }
      for (const [content, suffix] of [
        [' \ufeff\r\n', 'INVALID'],
        ['bad\u0007', 'INVALID'],
        ['x'.repeat(65_537), 'TOO_LARGE'],
      ]) {
        for (const kind of ['role', 'preamble']) {
          const args =
            kind === 'role'
              ? ['role', 'set', content, '--identity', 'Alice']
              : ['preamble', 'set', 'Alice', content];
          const result = await runCli(sandbox, [...args, '--json']);
          expect(result.status).toBe(1);
          expectError(result, `${kind.toUpperCase()}_INPUT_${suffix}`);
        }
      }
      expect(await json(sandbox, ['role', 'show', '--identity', 'Alice'])).toEqual(initial);
      const database = new Database(sandbox.database, { readonly: true });
      try {
        expect(database.prepare('SELECT name FROM identities').all()).toEqual([{ name: 'Alice' }]);
      } finally {
        database.close();
      }
      expect(readFileSync(log, 'utf8')).toBe('\n');
    });
  });

  it('reads only bounded regular role files and preserves state on every rejection', async () => {
    await withSandbox(async (sandbox) => {
      const log = await calibrateTmuxTripwire(sandbox);
      await json(sandbox, ['identity', 'create', 'Alice']);
      const file = path.join(sandbox.root, 'role.txt');
      const text = 'é'.repeat(32_768);
      writeFileSync(file, text);
      const original = await json(sandbox, ['role', 'set', '--file', file, '--identity', 'Alice']);
      expect(original).toMatchObject({ role: { content: text } });
      const fifo = path.join(sandbox.root, 'role.fifo');
      execFileSync('mkfifo', [fifo]);
      for (const [source, content, code] of [
        [file, Buffer.from([0xff]), 'ROLE_INPUT_INVALID'],
        [file, Buffer.alloc(65_537, 120), 'ROLE_INPUT_TOO_LARGE'],
        [sandbox.root, null, 'ROLE_FILE_ERROR'],
        [fifo, null, 'ROLE_FILE_ERROR'],
        [path.join(sandbox.root, 'missing'), null, 'ROLE_FILE_ERROR'],
      ] as const) {
        if (content) writeFileSync(source, content);
        const result = await runCli(
          sandbox,
          ['role', 'set', '--file', source, '--identity', 'Alice', '--json'],
          { deadlineMs: 2_000 }
        );
        expect(result.status).toBe(1);
        expectError(result, code);
        expect(await json(sandbox, ['role', 'show', '--identity', 'Alice'])).toEqual(original);
      }
      expect(readFileSync(log, 'utf8')).toBe('\n');
    });
  });
});
