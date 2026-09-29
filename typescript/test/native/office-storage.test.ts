import { existsSync, readdirSync } from 'node:fs';
import path from 'node:path';
import Database from 'better-sqlite3';
import { expect, it } from 'vitest';
import { expectError, parseWholeStdout, runCli, withSandbox } from '../support/cli-process.js';
import { createArtifact } from '../support/native-artifact.js';

const count = (database: string, table: string): number => {
  const connection = new Database(database, { readonly: true, fileMustExist: true });
  try {
    return (connection.prepare(`SELECT count(*) AS count FROM ${table}`).get() as { count: number })
      .count;
  } finally {
    connection.close();
  }
};

it(
  'migrates Office data only with consent and keeps it visible through the CLI and local service',
  { timeout: 180_000 },
  async () => {
    await withSandbox(async (sandbox) => {
      const prefix = path.join(sandbox.root, 'isolated office');
      const run = (args: string[]) => runCli(sandbox, ['office', '--prefix', prefix, ...args]);
      const office = async (args: string[]) => {
        const result = await run([...args, '--json']);
        expect(result.status, result.stdout + result.stderr).toBe(0);
        return parseWholeStdout(result);
      };
      const artifact = await createArtifact(sandbox, '0.1.0-alpha.4', new Uint8Array(), 'office');
      await office([
        'install',
        '--yes',
        '--archive',
        artifact.archive,
        '--manifest',
        artifact.manifest,
      ]);
      // A fresh install holds only seeded catalogs: nothing to move, no hint.
      const fresh = await run(['status']);
      expect(fresh.status, fresh.stderr).toBe(0);
      expect(fresh.stderr).not.toContain('migration available');
      const post = await office([
        'board',
        'post',
        '--general',
        '--owner',
        '--title',
        'Before',
        '--body',
        'Kept across the move',
      ]);
      const bea = await runCli(sandbox, ['identity', 'create', 'Bea', '--json']);
      expect(bea.status, bea.stdout).toBe(0);
      const beaId = JSON.parse(bea.stdout).identity.id as string;
      await office([
        'board',
        'post',
        '--general',
        '--identity',
        'Bea',
        '--title',
        'By Bea',
        '--body',
        'Signed',
      ]);
      const officeDatabase = path.join(sandbox.globalDir, 'office', 'office.db');
      const backups = path.join(sandbox.globalDir, 'backups');

      const human = await run(['status']);
      expect(human.status, human.stderr).toBe(0);
      expect(human.stderr).toContain(
        "Office storage migration available: run 'tmt office storage migrate' (the current storage keeps working until you do)."
      );
      const plan = await office(['storage', 'status']);
      expect(plan).toMatchObject({
        state: 'pending',
        source: sandbox.database,
        destination: officeDatabase,
        serviceRunning: false,
      });
      expect(plan.officeRows as number).toBeGreaterThan(0);

      // Without a terminal, migration needs --yes and changes nothing.
      expectError(await run(['storage', 'migrate', '--json']), 'OFFICE_CONSENT_REQUIRED');
      expect(existsSync(officeDatabase)).toBe(false);
      expect(existsSync(backups)).toBe(false);

      const migrated = await office(['storage', 'migrate', '--yes']);
      expect(migrated).toMatchObject({ state: 'switched', database: officeDatabase });
      const backup = (migrated.backup as { directory: string }).directory;
      expect(readdirSync(backups)).toEqual([path.basename(backup)]);
      expect(existsSync(path.join(backup, 'tmux-team.db'))).toBe(true);
      expect(await office(['storage', 'status'])).toMatchObject({ state: 'switched' });
      const status = await run(['storage', 'status']);
      expect(status.stdout).toContain(`Office storage: ${officeDatabase} (migrated `);
      expect(status.stdout).toContain('Backups are kept until you remove them.');
      expect((await run(['status'])).stderr).not.toContain('migration available');

      // Reads and writes now use office.db; the retained core copy is untouched.
      const retained = count(sandbox.database, 'office_board_entries');
      const shown = await office(['board', 'show', post.threadId as string]);
      expect(shown.thread).toMatchObject({ title: 'Before', body: 'Kept across the move' });
      await office(['board', 'post', '--general', '--owner', '--title', 'After', '--body', 'New']);
      expect(count(officeDatabase, 'office_board_entries')).toBe(retained + 1);
      expect(count(sandbox.database, 'office_board_entries')).toBe(retained);

      // The browser's local API serves the migrated rows after a restart.
      // Retired after the move: the service reconciles before serving.
      const retired = await runCli(sandbox, ['rm', 'Bea', '--force', '--json']);
      expect(retired.status, retired.stdout).toBe(0);
      const started = await office(['start']);
      const marker = new Database(officeDatabase, { readonly: true });
      try {
        expect(marker.prepare('SELECT identity_id FROM office_retired_identities').all()).toEqual([
          { identity_id: beaId },
        ]);
      } finally {
        marker.close();
      }
      try {
        const url = new URL(started.url as string);
        const token = new URLSearchParams(url.hash.slice(1)).get('token');
        const response = await fetch(new URL('/api/v1/local/board/threads/show', url.origin), {
          method: 'POST',
          headers: {
            authorization: `Bearer ${token}`,
            origin: url.origin,
            'content-type': 'application/json',
          },
          body: JSON.stringify({ threadId: post.threadId, replyLimit: 10, replyCursor: null }),
        });
        expect(response.status).toBe(200);
        expect(((await response.json()) as { thread: { body: string } }).thread.body).toBe(
          'Kept across the move'
        );
      } finally {
        await office(['stop']);
      }
    });
  }
);
