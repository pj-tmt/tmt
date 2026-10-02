import { existsSync, mkdirSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { writeExecutable } from '../support/executable-fixture.mjs';
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
      // Retained-install fixture: public Office acquisition is frozen.
      const artifact = await createArtifact(sandbox, '0.1.0-alpha.4', new Uint8Array(), 'office');
      const installed = await runCli(sandbox, [
        '__native-install',
        '--product',
        'office',
        '--channel',
        'alpha',
        '--prefix',
        prefix,
        '--json',
        '--archive',
        artifact.archive,
        '--manifest',
        artifact.manifest,
      ]);
      expect(installed.status, installed.stdout + installed.stderr).toBe(0);
      // A fresh install holds only seeded catalogs: nothing to move, no hint.
      const fresh = await run(['status']);
      expect(fresh.status, fresh.stderr).toBe(0);
      expect(fresh.stderr).not.toContain('migration available');
      // Legacy user data: rows written by a pre-extraction Office in the shared core file.
      const bea = await runCli(sandbox, ['identity', 'create', 'Bea', '--json']);
      expect(bea.status, bea.stdout).toBe(0);
      const beaId = JSON.parse(bea.stdout).identity.id as string;
      const post = { threadId: '11111111-1111-4111-8111-111111111111' };
      const legacy = new Database(sandbox.database);
      try {
        const entry = legacy.prepare(
          `INSERT INTO office_board_entries
             (id, thread_id, is_root, category_kind, category_id, author_kind, author_id,
              author_name, revision, deleted, created_sequence, activity_sequence,
              created_at_ms, updated_at_ms, title, body)
           VALUES (?, ?, 1, 'general', NULL, ?, ?, ?, 1, 0, ?, ?, 1, 1, ?, ?)`
        );
        entry.run(
          post.threadId,
          post.threadId,
          'owner',
          'owner',
          null,
          1,
          1,
          'Before',
          'Kept across the move'
        );
        entry.run(
          '22222222-2222-4222-8222-222222222222',
          '22222222-2222-4222-8222-222222222222',
          'identity',
          beaId,
          'Bea',
          2,
          2,
          'By Bea',
          'Signed'
        );
        legacy.prepare('UPDATE office_board_state SET revision = 2, next_sequence = 3').run();
      } finally {
        legacy.close();
      }
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

it(
  'hands the invoking core to one-shot operations and the service even under the tmux-team name',
  { timeout: 180_000 },
  async () => {
    await withSandbox(async (sandbox) => {
      // A different `tmt` earlier on PATH must never be used to reach core.
      const decoyDirectory = path.join(sandbox.root, 'decoy');
      const marker = path.join(sandbox.root, 'decoy-used');
      mkdirSync(decoyDirectory);
      writeExecutable(
        path.join(decoyDirectory, 'tmt'),
        `#!/bin/sh\necho used >> '${marker}'\nexit 1\n`,
        0o755
      );
      const linkDirectory = path.join(sandbox.root, 'link');
      mkdirSync(linkDirectory);
      const link = path.join(linkDirectory, 'tmux-team');
      writeExecutable(
        link,
        readFileSync(sandbox.cli.executable),
        statSync(sandbox.cli.executable).mode & 0o777
      );
      const named = {
        ...sandbox,
        cli: { ...sandbox.cli, executable: link },
        env: { ...sandbox.env, PATH: `${decoyDirectory}:${sandbox.env.PATH ?? ''}` },
      };
      const prefix = path.join(sandbox.root, 'isolated office');
      // macOS arm64 subprocess maxima: 1.884 s quiet / 1.945 s with the tooling suite.
      // Match the Office world fixture's 15 s bound, leaving room for the service's
      // own 5 s readiness wait plus the core/companion launches and response.
      const run = (args: string[]) =>
        runCli(named, ['office', '--prefix', prefix, ...args], { deadlineMs: 15_000 });
      // Retained-install fixture: public Office acquisition is frozen.
      const artifact = await createArtifact(sandbox, '0.1.0-alpha.4', new Uint8Array(), 'office');
      const installed = await runCli(
        named,
        [
          '__native-install',
          '--product',
          'office',
          '--channel',
          'alpha',
          '--prefix',
          prefix,
          '--archive',
          artifact.archive,
          '--manifest',
          artifact.manifest,
          '--json',
        ],
        { deadlineMs: 15_000 }
      );
      expect(installed.status, installed.stdout + installed.stderr).toBe(0);
      const posted = await run([
        'board',
        'post',
        '--general',
        '--owner',
        '--title',
        'Named',
        '--body',
        'Through tmux-team',
        '--json',
      ]);
      expect(posted.status, posted.stdout + posted.stderr).toBe(0);
      const started = await run(['start', '--json']);
      expect(started.status, started.stdout + started.stderr).toBe(0);
      const stopped = await run(['stop', '--json']);
      expect(stopped.status, stopped.stdout + stopped.stderr).toBe(0);
      expect(existsSync(marker), 'the PATH tmt was used to reach core').toBe(false);
    });
  }
);
