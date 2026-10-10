import { mkdirSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import { pairBrowser, startDoor } from './harness/browser.js';
import {
  composeChat,
  createPage,
  freePort,
  openChat,
  openPage,
  run,
  sendChat,
} from './harness/ask.js';
import { until } from './harness/process.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

const messageOrder = (page: Page) =>
  page
    .getByTestId('chat-panel')
    .getByTestId('comment-entry')
    .evaluateAll((nodes) => nodes.map((node) => (node as HTMLElement).dataset.messageId));

// #2442: Chat comments sent back to back keep their submission order in the live panel, after
// reload, on a second device and in the export, with no timestamp deciding any of them.
test.afterEach(disposeActiveWorlds);
test('Chat comments keep one causal order live, after reload, on another device and in the export', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('chat-order-agent');
    const author = await pairBrowser(world, 'order-author', { talk: true });
    const viewer = await pairBrowser(world, 'order-viewer', { talk: true });
    const created = createPage(world, 'Chat order', '<h1>Chat order</h1><p>Two quick turns.</p>');
    const page = await openPage(door, author, created);
    const turns = ['We will run the pilot', 'The draft announcement', 'Then a third turn'];
    const sent: string[] = [];
    for (const [index, turn] of turns.entries()) {
      sent.push((await sendChat(page, await composeChat(page, agent, turn))).operationId);
      await until(() => agent.received().length === index + 1, `turn ${index + 1} delivered`);
    }

    const directory = mkdtempSync(path.join(tmpdir(), 'colab-chat-order-'));
    try {
      const exported = JSON.parse(
        run(world, world.binaries.colab, ['export', created.pageId, '--dir', directory, '--json']),
      ) as { directory: string };
      const conversations = JSON.parse(
        readFileSync(path.join(exported.directory, 'conversations.json'), 'utf8'),
      );
      // Submission order is the order of the operations' own message IDs.
      const submitted = sent.map(
        (operationId) =>
          conversations.asks.find(
            (ask: { operationId: string }) => ask.operationId === operationId,
          ).messageIds[0],
      );
      const [thread] = conversations.threads;
      expect(thread.comments.map((comment: { id: string }) => comment.id)).toEqual(submitted);
      expect(thread.comments.map((comment: { sequence: string }) => comment.sequence)).toEqual([
        '1',
        '2',
        '3',
      ]);
      const reading = readFileSync(path.join(exported.directory, 'conversations.md'), 'utf8');
      const positions = turns.map((turn) => reading.indexOf(turn));
      expect(positions.every((position) => position >= 0)).toBe(true);
      expect(positions).toEqual([...positions].sort((a, b) => a - b));

      // Optional look capture for the UX check: the same order live and after reload.
      const captures = process.env.COLAB_2442_CAPTURE_DIR;
      if (captures) mkdirSync(captures, { recursive: true });
      const capture = async (name: string) => {
        if (captures) await page.screenshot({ path: path.join(captures, `${name}.png`) });
      };
      await expect.poll(() => messageOrder(page)).toEqual(submitted);
      for (const [width, theme] of [
        [1440, 'light'],
        [390, 'dark'],
      ] as const) {
        await page.setViewportSize({ width, height: 900 });
        await page.evaluate((value) => (document.documentElement.dataset.theme = value), theme);
        await capture(`chat-order-live-${width}-${theme}`);
      }
      await page.reload();
      await openChat(page);
      await expect.poll(() => messageOrder(page)).toEqual(submitted);
      for (const [width, theme] of [
        [1440, 'light'],
        [390, 'dark'],
      ] as const) {
        await page.setViewportSize({ width, height: 900 });
        await page.evaluate((value) => (document.documentElement.dataset.theme = value), theme);
        await capture(`chat-order-reload-${width}-${theme}`);
      }

      const other = await openPage(door, viewer, created);
      await openChat(other);
      await expect.poll(() => messageOrder(other)).toEqual(submitted);
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });
});
