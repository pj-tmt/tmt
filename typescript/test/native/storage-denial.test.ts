import { chmodSync, mkdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { describe, expect, it } from 'vite-plus/test';
import { expectError, runCli, withSandbox } from '../support/cli-process.js';

const identityId = '00000000-0000-4000-8000-000000000001';
const cases = [
  { args: ['ls', '--json'], code: 'IDENTITY_ERROR' },
  { args: ['identity', 'create', 'Ada', '--json'], code: 'IDENTITY_ERROR' },
  { args: ['identity', 'status', 'show', '--identity', 'Ada', '--json'], code: 'IDENTITY_ERROR' },
  { args: ['x', '--identity', 'Ada', '--json'], code: 'X_ERROR' },
  {
    args: ['x', 'listen', '--identity', 'Ada', '--timeout', '1ms', '--debounce', '1ms', '--json'],
    code: 'X_ERROR',
  },
  { args: ['role', 'show', '--identity', 'Ada', '--json'], code: 'ROLE_ERROR' },
  { args: ['preamble', 'show', '--json'], code: 'PREAMBLE_ERROR' },
  { args: ['notes', 'path', '--identity', 'Ada', '--json'], code: 'NOTES_IO_ERROR' },
  { args: ['room', 'ls', '--json'], code: 'STORAGE_UNAVAILABLE' },
  {
    args: ['room', 'send', 'Review', 'never queued', '--identity', 'Ada', '--json'],
    code: 'STORAGE_UNAVAILABLE',
  },
  {
    args: ['api'],
    stdin: JSON.stringify({ version: 1, operation: 'changes.cursor', input: {} }),
    code: 'API_UNAVAILABLE',
  },
  {
    args: ['api'],
    stdin: JSON.stringify({ version: 1, operation: 'notes.read', input: { identityId } }),
    code: 'NOTEBOOK_UNAVAILABLE',
  },
];

describe('typed storage-open denial across core commands', () => {
  it('names the denied directory without changing bytes or modes, then recovers', async () => {
    await withSandbox(async (sandbox) => {
      const created = await runCli(sandbox, ['identity', 'create', 'Ada', '--json']);
      expect(created.status).toBe(0);
      const id = JSON.parse(created.stdout).identity.id;
      expect((await runCli(sandbox, ['room', 'create', 'Review', '--json'])).status).toBe(0);
      expect(
        (await runCli(sandbox, ['room', 'join', 'Review', '--identity', 'Ada', '--json'])).status
      ).toBe(0);
      const notes = await runCli(sandbox, ['notes', 'path', '--identity', 'Ada', '--json']);
      expect(notes.status).toBe(0);
      const notebook = JSON.parse(notes.stdout).path;
      writeFileSync(notebook, 'Retain exact notebook bytes.\r\n');
      const before = readFileSync(sandbox.database);
      const fileMode = statSync(sandbox.database).mode;
      chmodSync(sandbox.globalDir, 0o500);
      try {
        for (const item of cases) {
          const stdin = item.stdin?.replace(identityId, id);
          const failed = await runCli(sandbox, item.args, { stdin });
          expect(failed.status).toBe(1);
          expect(failed.stderr).toBe('');
          const body = expectError(failed, 'STORAGE_NOT_WRITABLE');
          expect(Object.keys(body)).toEqual(['error']);
          expect(body).toMatchObject({
            error: { message: expect.stringContaining(sandbox.globalDir) },
          });
          expect(body).toMatchObject({ error: { message: expect.stringContaining('sandbox') } });
          expect(statSync(sandbox.globalDir).mode & 0o777).toBe(0o500);
          expect(statSync(sandbox.database).mode).toBe(fileMode);
          expect(readFileSync(sandbox.database)).toEqual(before);
          expect(readFileSync(notebook, 'utf8')).toBe('Retain exact notebook bytes.\r\n');
        }
        // Discovery remains independent of storage even when it cannot be opened.
        for (const operation of ['capabilities', 'storage.root']) {
          expect(
            (
              await runCli(sandbox, ['api'], {
                stdin: JSON.stringify({ version: 1, operation, input: {} }),
              })
            ).status
          ).toBe(0);
        }
      } finally {
        chmodSync(sandbox.globalDir, 0o700);
      }
      for (const item of cases) {
        expect(
          (await runCli(sandbox, item.args, { stdin: item.stdin?.replace(identityId, id) })).status
        ).toBe(0);
      }
    });
  });

  it('preserves human launch-state refusal and resumes normal forgotten-session lookup', async () => {
    await withSandbox(async (sandbox) => {
      expect((await runCli(sandbox, ['identity', 'create', 'Ada', '--json'])).status).toBe(0);
      const before = readFileSync(sandbox.database);
      chmodSync(sandbox.globalDir, 0o500);
      try {
        const denied = await runCli(sandbox, ['resume', '--forget', 'Ada']);
        expect(denied.status).toBe(1);
        expect(denied.stdout).toBe('');
        expect(denied.stderr).toContain(sandbox.globalDir);
        expect(denied.stderr).toContain('No remembered session was changed');
        expect(readFileSync(sandbox.database)).toEqual(before);
      } finally {
        chmodSync(sandbox.globalDir, 0o700);
      }
      expect((await runCli(sandbox, ['resume', '--forget', 'Ada'])).status).toBe(0);
    });
  });

  it.each(['corrupt', 'directory'] as const)(
    'preserves prior codes for a %s database',
    async (kind) => {
      await withSandbox(async (sandbox) => {
        mkdirSync(sandbox.globalDir, { recursive: true });
        if (kind === 'corrupt') writeFileSync(sandbox.database, 'not a SQLite database');
        else mkdirSync(sandbox.database);
        for (const item of cases) {
          const failed = await runCli(sandbox, item.args, { stdin: item.stdin });
          expect(failed.status).toBe(1);
          expect(failed.stderr).toBe('');
          expectError(failed, item.code);
        }
        if (kind === 'corrupt')
          expect(readFileSync(sandbox.database, 'utf8')).toBe('not a SQLite database');
        else expect(statSync(sandbox.database).isDirectory()).toBe(true);
      });
    }
  );
});
