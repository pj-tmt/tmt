import Database from 'better-sqlite3';
import { chmodSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { expectError, fileSnapshot, runCli, withSandbox } from '../support/cli-process.js';
import { calibrateTmuxTripwire } from './tmux-tripwire.js';

describe('native check process preflight', () => {
  it('reports a denied private tmux socket without preparing check or talk effects', async () => {
    await withSandbox(async (sandbox) => {
      const directory = path.join(sandbox.root, 'denied-tmux');
      mkdirSync(directory);
      const executable = path.join(directory, 'tmux');
      sandbox.env.PATH = `${directory}${path.delimiter}${sandbox.env.PATH ?? ''}`;
      for (const reason of ['Permission denied', 'Operation not permitted']) {
        writeFileSync(
          executable,
          `#!/bin/sh\nprintf "error connecting to /tmp/private.sock (${reason})\\n" >&2\nexit 1\n`
        );
        chmodSync(executable, 0o755);
        for (const args of [
          ['check', '%14', '--json'],
          ['talk', '%14', 'must not send', '--detach', '--json'],
        ]) {
          const failed = await runCli(sandbox, args);
          expect(failed.status).toBe(1);
          expectError(failed, 'TMUX_PERMISSION_DENIED');
          expect(failed.stderr).toBe('');
        }
      }
      const database = new Database(sandbox.database, { readonly: true });
      try {
        expect(database.prepare('SELECT COUNT(*) AS count FROM request_attempts').get()).toEqual({
          count: 0,
        });
      } finally {
        database.close();
      }
    });
  });

  it('validates configuration before any tmux call or storage creation', async () => {
    for (const local of [false, true]) {
      await withSandbox(async (sandbox) => {
        const log = await calibrateTmuxTripwire(sandbox);
        mkdirSync(sandbox.globalDir, { recursive: true });
        writeFileSync(local ? sandbox.localConfig : sandbox.globalConfig, '{ invalid config');
        const before = fileSnapshot(sandbox.root);
        const result = await runCli(sandbox, ['check', '%14', '--json']);
        expect(result.status).toBe(1);
        expectError(result, 'CONFIG_ERROR');
        expect(result.stderr).toBe('');
        expect(existsSync(sandbox.database)).toBe(false);
        expect(readFileSync(log, 'utf8')).toBe('\n');
        expect(fileSnapshot(sandbox.root)).toEqual(before);
      });
    }
  });

  it('does not create unknown names or perform endpoint IO for lookup-only misses', async () => {
    await withSandbox(async (sandbox) => {
      const log = await calibrateTmuxTripwire(sandbox);
      for (const name of ['NeverCreated', 'bad\nname']) {
        const result = await runCli(sandbox, ['read', name, '--json']);
        expect(result.status).toBe(3);
        expectError(result, 'NAME_NOT_FOUND');
        expect(result.stderr).toBe('');
      }
      expect(readFileSync(log, 'utf8')).toBe('\n');
      const database = new Database(sandbox.database, { readonly: true });
      try {
        expect(database.prepare('SELECT COUNT(*) AS count FROM identities').get()).toEqual({
          count: 0,
        });
      } finally {
        database.close();
      }
    });
  });
});
