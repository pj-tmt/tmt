import Database from 'better-sqlite3';
import { existsSync, lstatSync, readlinkSync, renameSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import {
  expectJsonSuccess,
  parseWholeStdout,
  runCli,
  withSandbox,
} from '../support/cli-process.js';

describe('Core database basename (#2292)', () => {
  it('keeps a former-name store working, renames it only when nothing holds it, and leaves a link', async () => {
    await withSandbox(async (sandbox) => {
      const created = await runCli(sandbox, ['identity', 'create', 'Legacy', '--json']);
      expect(created.status).toBe(0);
      const current = path.join(sandbox.globalDir, 'tmt.db');
      const former = path.join(sandbox.globalDir, 'tmux-team.db');
      expect(existsSync(current)).toBe(true);
      // Lay the closed store out as a release from before the rename did.
      for (const suffix of ['', '-wal', '-shm']) {
        if (existsSync(current + suffix)) renameSync(current + suffix, former + suffix);
      }

      const before = await runCli(sandbox, ['identity', 'list', '--json']);
      expect(before.status).toBe(0);
      expect(existsSync(current), 'no empty database is created beside the old name').toBe(false);

      // A process holding the database keeps its name, untouched.
      const holder = new Database(former);
      holder.pragma('journal_mode = WAL');
      expectJsonSuccess(await runCli(sandbox, ['__native-rename-database', '--json']), {
        status: 'in-use',
      });
      expect(lstatSync(former).isFile()).toBe(true);
      expect(existsSync(current)).toBe(false);
      holder.close();

      expectJsonSuccess(await runCli(sandbox, ['__native-rename-database', '--json']), {
        status: 'renamed',
      });
      expect(lstatSync(current).isFile()).toBe(true);
      expect(lstatSync(former).isSymbolicLink()).toBe(true);
      expect(readlinkSync(former)).toBe('tmt.db');
      const after = await runCli(sandbox, ['identity', 'list', '--json']);
      expect(parseWholeStdout(after)).toEqual(parseWholeStdout(before));
      expectJsonSuccess(await runCli(sandbox, ['__native-rename-database', '--json']), {
        status: 'current',
      });
    });
  });

  it('creates the new name for a new store', async () => {
    await withSandbox(async (sandbox) => {
      expect((await runCli(sandbox, ['identity', 'create', 'Fresh', '--json'])).status).toBe(0);
      expect(existsSync(path.join(sandbox.globalDir, 'tmt.db'))).toBe(true);
      expect(existsSync(path.join(sandbox.globalDir, 'tmux-team.db'))).toBe(false);
      expectJsonSuccess(await runCli(sandbox, ['__native-rename-database', '--json']), {
        status: 'current',
      });
    });
  });
});
