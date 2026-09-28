import { randomUUID } from 'node:crypto';
import Database from '../../../../../../typescript/test/support/sqlite-oracle.js';
import { expect, test } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { openOfficeObjects } from './office-navigation.js';

function boardState(file: string) {
  const database = new Database(file, { readonly: true });
  try {
    return {
      entries: database
        .prepare('SELECT * FROM office_board_entries ORDER BY created_sequence')
        .all(),
      state: database.prepare('SELECT * FROM office_board_state').all(),
      operations: database
        .prepare('SELECT * FROM office_board_operations ORDER BY operation_id')
        .all(),
    };
  } finally {
    database.close();
  }
}

test('offline CLI posts, browser moderation and stale edits share durable board state', async ({
  page,
}, info) => {
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    const cli = async (args: string[]) => {
      const result = await runCli(sandbox, [...args, '--json'], { deadlineMs: 30_000 });
      expect(result.status, result.stdout).toBe(0);
      return JSON.parse(result.stdout);
    };
    const office = (args: string[]) => cli(['office', '--prefix', prefix, ...args]);
    const repository = 'example.com/Organization/Office-Discussion-Board-With-A-Long-Category';
    const git = { ...sandbox, cli: { executable: 'git', args: [] } };
    for (const args of [
      ['init', '--quiet'],
      ['remote', 'add', 'origin', `https://${repository}.git`],
    ]) {
      const result = await runCli(git, args);
      expect(result.status, result.stderr).toBe(0);
    }
    await cli(['identity', 'create', 'Board author']);
    expect((await office(['status'])).service.running).toBe(false);
    const post = await office([
      'board',
      'post',
      '--general',
      '--identity',
      'Board author',
      '--title',
      'Persistent review',
      '--body',
      'Created while Office is stopped.',
    ]);
    const hostile = '<script>window.__tmtBoardScriptRan = true</script>';
    const repositoryPost = await office([
      'board',
      'post',
      '--repo',
      'origin',
      '--identity',
      'Board author',
      '--title',
      'Repository review',
      '--body',
      hostile,
    ]);
    expect((await office(['status'])).service.running).toBe(false);
    const stored = boardState(sandbox.database);
    expect(stored.entries).toEqual([
      expect.objectContaining({
        id: post.entryId,
        thread_id: post.threadId,
        revision: 1,
        title: 'Persistent review',
        body: 'Created while Office is stopped.',
        deleted: 0,
      }),
      expect.objectContaining({ id: repositoryPost.entryId, body: hostile, deleted: 0 }),
    ]);
    expect(stored.state).toEqual([expect.objectContaining({ revision: 2 })]);
    let started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    const board = page.getByRole('dialog', { name: 'Discussion board', exact: true });
    const traffic: string[] = [];
    page.on('request', (request) => traffic.push(new URL(request.url()).hostname));
    const open = async () => {
      await page.goto(started.url);
      await openOfficeObjects(page);
      await page.getByRole('button', { name: 'Open discussion board', exact: true }).click();
      await expect(board).toBeVisible();
    };
    try {
      await page.setViewportSize({ width: 1440, height: 1000 });
      await open();
      await expect(board.getByRole('heading', { name: 'Persistent review' })).toBeVisible();
      await expect(board.getByText(/Board author · revision 1/)).toBeVisible();
      await expect(board.getByRole('button', { name: /Repository review/ })).toHaveCount(0);
      const reply = board.locator('.board-reply-form');
      await reply.getByLabel('Message').fill('Reply from the Office owner.');
      await reply.getByRole('button', { name: 'Reply as owner' }).click();
      await expect(board.getByText('Reply from the Office owner.')).toBeVisible();

      await board.getByRole('button', { name: 'New post', exact: true }).click();
      await board.getByLabel('Title', { exact: true }).fill('Owner follow-up');
      const compose = board.locator('form').filter({
        has: page.getByRole('heading', { name: 'New post', exact: true }),
      });
      await expect(compose).toBeVisible();
      await compose.getByRole('textbox', { name: 'Message', exact: true }).fill('Before editing.');
      await board.getByRole('button', { name: 'Post as owner' }).click();
      const newThread = board.getByRole('button', { name: /^Owner follow-up / });
      await expect(newThread).toBeVisible();
      // Posting refreshes the index without implicitly disposing the selected
      // thread's forms. Navigation remains an explicit user action.
      await expect(
        board.getByRole('heading', { name: 'Persistent review', exact: true })
      ).toBeVisible();
      await newThread.click();
      await expect(
        board.getByRole('heading', { name: 'Owner follow-up', exact: true })
      ).toBeVisible();
      await board.getByRole('button', { name: 'Edit', exact: true }).click();
      await board.getByLabel('Title', { exact: true }).fill('Owner follow-up edited');
      await board.locator('.board-entry form').getByLabel('Message').fill('Durable owner edit.');
      await board.getByRole('button', { name: 'Save edit' }).click();
      await expect(board.getByRole('heading', { name: 'Owner follow-up edited' })).toBeVisible();

      const categories = board.getByRole('group', { name: 'Category' });
      await categories.getByRole('button', { name: repository, exact: true }).click();
      await expect(board.getByRole('heading', { name: 'Repository review' })).toBeVisible();
      await expect(board.getByText(hostile, { exact: true })).toBeVisible();
      await expect(board.getByRole('button', { name: /Persistent review/ })).toHaveCount(0);
      await expect(page.locator('script').filter({ hasText: '__tmtBoardScriptRan' })).toHaveCount(
        0
      );
      expect(await page.evaluate(() => Reflect.get(window, '__tmtBoardScriptRan'))).toBeUndefined();
      await page.screenshot({ path: info.outputPath('board-administration-desktop.png') });
      await page.setViewportSize({ width: 390, height: 844 });
      await board.getByRole('button', { name: /Repository review/ }).click();
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        390
      );
      await page.screenshot({ path: info.outputPath('board-administration-narrow.png') });
      page.once('dialog', (dialog) => dialog.accept());
      await board.getByRole('button', { name: 'Moderate delete' }).click();
      await expect(board.getByText('Deleted entry')).toBeVisible();

      const after = boardState(sandbox.database);
      expect(after.entries).toContainEqual(
        expect.objectContaining({
          id: repositoryPost.entryId,
          title: null,
          body: null,
          deleted: 1,
          author_kind: 'identity',
        })
      );
      const owner = after.entries.find(
        (value) => (value as { title: string }).title === 'Owner follow-up edited'
      ) as { id: string };
      expect(owner).toMatchObject({
        author_kind: 'owner',
        revision: 2,
        body: 'Durable owner edit.',
        deleted: 0,
      });
      expect(after.state).toEqual([expect.objectContaining({ revision: 6 })]);
      expect((await office(['board', 'show', owner.id])).thread).toMatchObject({
        revision: 2,
        body: 'Durable owner edit.',
        author: { kind: 'owner' },
      });

      const url = new URL(started.url);
      const token = new URLSearchParams(url.hash.slice(1)).get('token');
      const stale = await fetch(`${url.origin}/api/v1/local/board/entries/${owner.id}`, {
        method: 'PUT',
        headers: {
          Authorization: `Bearer ${token}`,
          Origin: url.origin,
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          title: 'Stale overwrite',
          body: 'Must not commit.',
          ifRevision: 1,
          operationId: randomUUID(),
        }),
        signal: AbortSignal.timeout(10_000),
      });
      expect(stale.status).toBe(409);
      expect(await stale.json()).toMatchObject({ error: 'BOARD_REVISION_CONFLICT' });
      expect(boardState(sandbox.database)).toEqual(after);

      // The same CLI remains useful with the web service stopped.
      await office(['stop']);
      const cliReply = await office([
        'board',
        'reply',
        post.threadId,
        '--identity',
        'Board author',
        '--body',
        'Reply from a fresh offline CLI process.',
      ]);
      expect(cliReply.revision).toBe(1);
      expect((await office(['status'])).service.running).toBe(false);
      const final = boardState(sandbox.database);
      expect(final.entries).toContainEqual(
        expect.objectContaining({
          id: cliReply.entryId,
          thread_id: post.threadId,
          body: 'Reply from a fresh offline CLI process.',
          author_kind: 'identity',
        })
      );
      started = await office(['start', '--port', String(await unusedLoopbackPort())]);
      await page.setViewportSize({ width: 1440, height: 1000 });
      await open();
      await board.getByRole('button', { name: /Persistent review/ }).click();
      await expect(board.getByText('Reply from a fresh offline CLI process.')).toBeVisible();
      await expect(board.getByText('Reply from the Office owner.')).toBeVisible();
      await board.getByRole('button', { name: /Owner follow-up edited/ }).click();
      await expect(board.getByText('Durable owner edit.')).toBeVisible();
      await categories.getByRole('button', { name: repository, exact: true }).click();
      await expect(board.getByText('Deleted entry')).toBeVisible();
      expect(boardState(sandbox.database)).toEqual(final);
      expect(new Set(traffic)).toEqual(new Set(['127.0.0.1']));
    } finally {
      expect(await office(['stop'])).toMatchObject({ running: false });
    }
  });
});
