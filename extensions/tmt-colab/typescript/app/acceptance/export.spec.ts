import { pageAction } from '../test/page-actions.js';
import { expect, test, type Page } from '@playwright/test';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { pairBrowser, startDoor } from './harness/browser.js';
import {
  annotationInput,
  createPage,
  freePort,
  openPage,
  run,
  selectInRenderer,
} from './harness/ask.js';
import { until } from './harness/process.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

const FILES = ['page.html', 'conversations.json', 'conversations.md', 'manifest.json'] as const;

/** Every file the parent's Export panel offers, downloaded through the real browser. */
async function browserExport(page: Page): Promise<Record<string, Buffer>> {
  await (await pageAction(page, 'Export page')).click();
  const panel = page.getByRole('region', { name: 'Export page' });
  await expect(panel.getByRole('button', { name: 'Download page.html' })).toBeEnabled();
  const files: Record<string, Buffer> = {};
  for (const name of FILES) {
    const pending = page.waitForEvent('download');
    await panel.getByRole('button', { name: new RegExp(`Download ${name}`) }).click();
    const received = await pending;
    expect(received.suggestedFilename()).toBe(name);
    expect(await received.failure()).toBeNull();
    files[name] = readFileSync((await received.path())!);
  }
  await expect(panel.getByRole('status')).toContainText('All downloads requested');
  await panel.getByRole('button', { name: 'Close export' }).click();
  return files;
}

test.afterEach(disposeActiveWorlds);
test('browser and CLI export the same two-writer discussion and Ask conversation', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('export-agent');
    const a = await pairBrowser(world, 'export-author');
    const b = await pairBrowser(world, 'export-replier');
    const html =
      '<h1>Export review</h1><p id="quote">A `tick` &amp; <em>🌍 exact quote</em> to discuss.</p>';
    const created = createPage(world, 'Export review', html);
    const first = await openPage(door, a, created);
    const second = await openPage(door, b, created);

    // Each writer explicitly sends one turn in the same anchored conversation.
    const body = '@' + agent.name + ' Please check ``` this & <b>that</b>.\nSecond line.';
    const follow = `@${agent.name} A reply from the second device.`;
    await selectInRenderer(first, '#quote');
    await first.getByTestId('selection-ask').click();
    const opening = await annotationInput(first.locator('.annotation-new'), agent.name);
    await opening.fill(body);
    await opening.press('Enter');
    await until(() => agent.received().length === 1, 'opening annotation delivered');
    const t1 = first.getByTestId('comment-thread').first();
    await expect(t1.getByTestId('ask-reply')).toBeVisible();
    const operationId = (await t1.getByTestId('ask-entry').getAttribute('data-operation-id'))!;
    const messageId = (await t1.getByTestId('comment-entry').getAttribute('data-message-id'))!;
    await (await pageAction(second, 'Comments')).click();
    await second.getByTestId('annotation-row').click();
    const t2 = second.getByTestId('comment-thread').first();
    await expect(t2).toHaveAttribute('data-anchor', 'attached');
    await expect(
      t2
        .locator(`[data-testid=ask-entry][data-operation-id="${operationId}"]`)
        .getByTestId('ask-reply'),
    ).toBeVisible();
    const reply = await annotationInput(t2, agent.name);
    await reply.fill(follow);
    await reply.press('Enter');
    await until(() => agent.received().length === 2, 'follow-up annotation delivered');
    await expect(t1.getByTestId('comment-entry')).toHaveCount(2);
    await expect(t1.getByTestId('ask-reply')).toHaveCount(2);
    await expect(t2.getByTestId('ask-reply')).toHaveCount(2);
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      2,
    );

    // Both surfaces export the same authorized view.
    const browser = await browserExport(first);
    const directory = mkdtempSync(path.join(tmpdir(), 'colab-export-'));
    try {
      const exported = JSON.parse(
        run(world, world.binaries.colab, ['export', created.pageId, '--dir', directory, '--json']),
      ) as { directory: string; files: { name: string }[] };
      expect(exported.files.map((file) => file.name)).toEqual([...FILES]);
      const native = Object.fromEntries(
        FILES.map((name) => [name, readFileSync(path.join(exported.directory, name))]),
      );
      for (const name of ['page.html', 'conversations.json', 'conversations.md'] as const)
        expect(browser[name].equals(native[name]), name).toBe(true);
      const manifests = [browser, native].map((files) =>
        JSON.parse(files['manifest.json'].toString('utf8')),
      );
      for (const manifest of manifests) delete manifest.exportedAtMs;
      expect(manifests[0]).toEqual(manifests[1]);
      expect(manifests[0].discussions).toEqual({
        included: true,
        scope: 'current-epoch',
        format: 'tmt-colab-conversations',
        version: 1,
      });

      const conversations = JSON.parse(native['conversations.json'].toString('utf8'));
      expect(conversations.threads).toHaveLength(1);
      const [thread] = conversations.threads;
      expect(thread.anchor.exact).toContain('exact quote');
      expect(thread.comments.map((c: { body: string }) => c.body).sort()).toEqual(
        [body, follow].sort(),
      );
      expect(new Set(thread.comments.map((c: { writer: string }) => c.writer)).size).toBe(2);
      expect(new Set(thread.comments.map((c: { deviceName: string }) => c.deviceName))).toEqual(
        new Set(['export-author', 'export-replier']),
      );
      expect(conversations.asks).toHaveLength(2);
      const ask = conversations.asks.find(
        (value: { operationId: string }) => value.operationId === operationId,
      );
      expect(ask).toMatchObject({
        operationId,
        state: 'accepted',
        thread: thread.id,
        messageIds: [messageId],
      });
      expect(ask.reply.body).toMatch(/^ask-reply:[0-9a-f]{16}$/);
      const reading = native['conversations.md'].toString('utf8');
      expect(reading).toContain(ask.reply.body);
      expect(reading).toContain('Second line.');
      // The body's backtick run cannot end its fence.
      expect(reading).toContain('````\n@' + agent.name + ' Please check ``` this');
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });
});
