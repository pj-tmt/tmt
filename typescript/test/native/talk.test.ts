import Database from 'better-sqlite3';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { describe, expect, it } from 'vite-plus/test';
import { expectError, fileSnapshot, runCli, withSandbox } from '../support/cli-process.js';
import { calibrateTmuxTripwire } from './tmux-tripwire.js';

describe('native talk preflight', () => {
  it('rejects invalid config and timing before storage or endpoint effects', async () => {
    await withSandbox(async (sandbox) => {
      const log = await calibrateTmuxTripwire(sandbox);
      for (const args of [
        ['talk', '%14', 'hello', '--timeout', '0'],
        ['talk', '%14', 'hello', '--delay', '-1'],
      ]) {
        const before = fileSnapshot(sandbox.root);
        const result = await runCli(sandbox, [...args, '--json']);
        expect(result.status).toBe(1);
        expectError(result, 'USAGE_ERROR');
        expect(fileSnapshot(sandbox.root)).toEqual(before);
      }
      mkdirSync(sandbox.globalDir, { recursive: true });
      writeFileSync(sandbox.globalConfig, '{ invalid');
      const before = fileSnapshot(sandbox.root);
      const result = await runCli(sandbox, ['talk', '%14', 'hello', '--json']);
      expect(result.status).toBe(1);
      expectError(result, 'CONFIG_ERROR');
      expect(existsSync(sandbox.database)).toBe(false);
      expect(fileSnapshot(sandbox.root)).toEqual(before);
      expect(readFileSync(log, 'utf8')).toBe('\n');
    });
  });

  it('does not invent unknown targets or prepare exchanges on lookup misses', async () => {
    await withSandbox(async (sandbox) => {
      const log = await calibrateTmuxTripwire(sandbox);
      for (const name of ['NeverCreated', 'invalid\nname']) {
        const result = await runCli(sandbox, ['talk', name, 'hello', '--json']);
        expect(result.status).toBe(3);
        expectError(result, 'NAME_NOT_FOUND');
        expect(result.stderr).toBe('');
      }
      expect(readFileSync(log, 'utf8')).toBe('\n');
      const database = new Database(sandbox.database, { readonly: true });
      try {
        for (const table of ['identities', 'request_responses', 'request_attempts']) {
          expect(database.prepare(`SELECT COUNT(*) AS count FROM ${table}`).get()).toEqual({
            count: 0,
          });
        }
      } finally {
        database.close();
      }
    });
  });
});
