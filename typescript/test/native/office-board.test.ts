import path from 'node:path';
import Database from 'better-sqlite3';
import { expect, it } from 'vitest';
import { expectError, parseWholeStdout, runCli, withSandbox } from '../support/cli-process.js';
import { createArtifact } from '../support/native-artifact.js';

it(
  'routes room discussions through the installed companion without a browser or membership ACL',
  { timeout: 120_000 },
  async () => {
    await withSandbox(async (sandbox) => {
      const prefix = path.join(sandbox.root, 'isolated office');
      const cli = async (operation: string, args: string[]) => {
        const result = await runCli(sandbox, [...args, '--json']).catch((cause: unknown) => {
          throw new Error(`Room discussion scenario failed during ${operation}.`, { cause });
        });
        expect(result.status, result.stdout + result.stderr).toBe(0);
        return parseWholeStdout(result);
      };
      const office = (args: string[]) =>
        cli(`office ${args[0] === 'board' ? `board ${args[1]}` : args[0]}`, [
          'office',
          '--prefix',
          prefix,
          ...args,
        ]);
      // Retained-install fixture: public Office acquisition is frozen.
      const artifact = await createArtifact(sandbox, '0.1.0-alpha.4', new Uint8Array(), 'office');
      await cli('internal Office fixture install', [
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
      ]);
      const design = (await cli('room create', ['room', 'create', 'Design'])).room as {
        id: string;
      };
      const planning = (await cli('room create', ['room', 'create', 'Planning'])).room as {
        id: string;
      };
      const operation = '44444444-4444-4444-8444-444444444444';
      const post = [
        'board',
        'post',
        '--room',
        'Design',
        '--owner',
        '--title',
        'Room only',
        '--body',
        'Exact body',
        '--operation-id',
        operation,
      ];
      const receipt = await office(post);
      expect(await office(post)).toEqual(receipt);
      const listed = await office(['board', 'list', '--room', design.id]);
      expect(listed.threads).toEqual([
        expect.objectContaining({
          id: receipt.entryId,
          category: { kind: 'room', roomId: design.id },
          title: 'Room only',
        }),
      ]);
      expect((await office(['board', 'list', '--general'])).threads).toEqual([]);
      expect((await office(['board', 'list', '--room', planning.id])).threads).toEqual([]);
      await office([
        'board',
        'reply',
        receipt.threadId as string,
        '--owner',
        '--body',
        'Reply in the same room',
      ]);
      const shown = await office(['board', 'show', receipt.threadId as string]);
      expect(shown.thread).toMatchObject({
        category: { kind: 'room', roomId: design.id },
        body: 'Exact body',
      });
      expect(shown.replies).toEqual([
        expect.objectContaining({
          category: { kind: 'room', roomId: design.id },
          body: 'Reply in the same room',
        }),
      ]);

      await cli('room create', ['room', 'create', 'Design']);
      expectError(
        await runCli(sandbox, [
          'office',
          '--prefix',
          prefix,
          'board',
          'list',
          '--room',
          'Design',
          '--json',
        ]),
        'ROOM_AMBIGUOUS'
      );
      expect((await office(['board', 'list', '--room', design.id])).threads).toHaveLength(1);
      expectError(
        await runCli(sandbox, [
          'office',
          '--prefix',
          prefix,
          'board',
          'list',
          '--room',
          'Missing',
          '--json',
        ]),
        'ROOM_NOT_FOUND'
      );
      expect((await office(['status'])).service).toMatchObject({ running: false });
      // Office rows live in office.db; membership and request history stay in core.
      const database = new Database(path.join(sandbox.globalDir, 'office', 'office.db'), {
        readonly: true,
      });
      database.exec(`ATTACH DATABASE '${sandbox.database.replaceAll("'", "''")}' AS core`);
      try {
        expect(
          database
            .prepare(
              'SELECT category_kind,category_id,is_root FROM office_board_entries ORDER BY created_sequence'
            )
            .all()
        ).toEqual([
          { category_kind: 'room', category_id: design.id, is_root: 1 },
          { category_kind: 'room', category_id: design.id, is_root: 0 },
        ]);
        expect(database.prepare('SELECT count(*) FROM office_board_operations').pluck().get()).toBe(
          2
        );
        expect(database.prepare('SELECT count(*) FROM core.request_attempts').pluck().get()).toBe(
          0
        );
        expect(
          database.prepare('SELECT count(*) FROM core.office_meeting_members').pluck().get()
        ).toBe(0);
      } finally {
        database.close();
      }
    });
  }
);
