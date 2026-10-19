import { pageAction } from '../test/page-actions.js';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { expect, test, type Locator, type Page } from '@playwright/test';
import { pairBrowser, startDoor } from './harness/browser.js';
import { composeChat, createPage, freePort, openPage, run, sendChat } from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';
import { text } from '../src/strings.js';

test.afterEach(disposeActiveWorlds);

const PNG = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==',
  'base64',
);

const sha = (value: Buffer) => createHash('sha256').update(value).digest('hex');
const download = async (page: Page, row: Locator) => {
  const saved = page.waitForEvent('download', { timeout: 5_000 });
  saved.catch(() => undefined);
  await row.getByRole('button', { name: text.attachmentDownload }).click();
  return fs.readFileSync((await (await saved).path())!);
};

// Connection acknowledges each received wire message only after its queued admission finishes.
// Receiving a checkpoint at the transport is insufficient: an earlier fold can still be pending.
function watchCheckpointAdmission(page: Page) {
  let writer = '';
  const sockets: { received: number; acknowledged: number; ownCuts: Map<string, number> }[] = [];
  page.on('websocket', (socket) => {
    if (!new URL(socket.url()).pathname.endsWith('/sync')) return;
    const progress = { received: 0, acknowledged: 0, ownCuts: new Map<string, number>() };
    sockets.push(progress);
    socket.on('framereceived', ({ payload }) => {
      progress.received++;
      const frame = JSON.parse(String(payload)) as {
        type: string;
        namespace?: string;
        streamId?: string;
      };
      if (frame.type === 'checkpoint' && frame.namespace === 'own' && frame.streamId)
        progress.ownCuts.set(frame.streamId, progress.received);
    });
    socket.on('framesent', ({ payload }) => {
      const frame = JSON.parse(String(payload)) as { type: string; device?: string };
      if (frame.type === 'hello' && frame.device) writer = frame.device;
      if (frame.type === 'ack') progress.acknowledged++;
    });
  });
  return {
    writer: () => writer,
    admitted: (stream: string) =>
      sockets.some((progress) => {
        const ordinal = progress.ownCuts.get(stream);
        return ordinal !== undefined && progress.acknowledged >= ordinal;
      }),
  };
}

// A combine changes the page revision. A browser that stayed open must keep reading the page's
// document files: their reads are fenced by the revision the page has now.
test('a document file still downloads and previews after the serve combined its stream under an open browser', async () => {
  test.setTimeout(300_000);
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const author = await pairBrowser(world, 'revision-parity');
    const colab = world.binaries.colab;
    const json = (args: string[]) => JSON.parse(run(world, colab, args)) as Record<string, any>;
    const created = createPage(world, 'Revision parity', '<h1>Revision parity</h1>');
    let source = '<h1>Revision parity</h1>';
    for (let i = 0; i < 4; i++) {
      source += `<p>${i}</p>`;
      run(world, colab, ['page', 'write', created.pageId, '--file', '-', '--json'], source);
    }
    const page = await openPage(door, author, created);
    await expect(
      page.frameLocator('iframe').getByRole('heading', { name: 'Revision parity', exact: true }),
    ).toBeVisible();
    const file = path.join(world.workspace, 'dot.png');
    fs.writeFileSync(file, PNG);
    json(['attachment', 'attach', created.pageId, file, '--json']);
    await (await pageAction(page, 'Files')).click();
    const row = page.getByTestId('file-row').filter({ hasText: 'dot.png' });
    await expect(row).toHaveCount(1, { timeout: 30_000 });
    expect(sha(await download(page, row))).toBe(sha(PNG));

    // The serve combines its own stream; this browser is not reloaded and learns nothing of it.
    const compacted = json(['page', 'compact', created.pageId, '--json']);
    expect(compacted.compacted).toBe(true);
    const revision = json(['page', 'read', created.pageId, '--json']).revision;
    expect(revision).toMatch(/^v1:/);

    expect(sha(await download(page, row))).toBe(sha(PNG));
    await row.getByRole('button', { name: text.attachmentPreview }).click();
    await expect(page.getByTestId('file-preview').locator('img')).toHaveAttribute(
      'src',
      /^data:image\/png;base64,/,
    );
  });
});

// The same holds when the checkpoint is another browser's: it combines its own stream, the serve
// stores and announces it, and this browser, still open, follows without a reload.
test('a document file still downloads and previews after another browser combined its own stream', async () => {
  test.setTimeout(300_000);
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('revision-parity-agent', { gated: true });
    const first = await pairBrowser(world, 'revision-parity-first');
    const second = await pairBrowser(world, 'revision-parity-second', { talk: true });
    const colab = world.binaries.colab;
    const created = createPage(world, 'Revision parity', '<h1>Revision parity</h1>', agent.pane);
    const open = await first.context.newPage();
    const reader = watchCheckpointAdmission(open);
    await open.goto(new URL(created.path, `${door.address}/`).href);
    await expect(
      open.frameLocator('iframe').getByRole('heading', { name: 'Revision parity', exact: true }),
    ).toBeVisible();
    const file = path.join(world.workspace, 'dot.png');
    fs.writeFileSync(file, PNG);
    run(world, colab, ['attachment', 'attach', created.pageId, file, '--json']);
    await (await pageAction(open, 'Files')).click();
    const row = open.getByTestId('file-row').filter({ hasText: 'dot.png' });
    await expect(row).toHaveCount(1, { timeout: 30_000 });
    expect(sha(await download(open, row))).toBe(sha(PNG));

    // The other browser makes changes on its own stream and combines them.
    const other = await second.context.newPage();
    const writer = watchCheckpointAdmission(other);
    await other.goto(new URL(created.path, `${door.address}/`).href);
    for (const question of ['first', 'second', 'third'])
      await sendChat(other, await composeChat(other, agent, `Question ${question}`));
    await expect(other.getByTestId('chat-panel').getByTestId('ask-entry')).toHaveCount(3);
    await other.getByRole('button', { name: 'Close Chat', exact: true }).click();
    await (await pageAction(other, 'About this page')).click();
    const section = other.locator('.page-size');
    await section.getByRole('button', { name: 'Compact page' }).click();
    await expect(section.getByRole('status')).toContainText(/^Combined \d+ changes? into one\./);

    // The other browser's status proves its store, not this browser's admission. Wait for
    // this exact writer's own-checkpoint notice and its queued ACK before starting Download's
    // unchanged event bound. The download then reopens once to take in the known cut.
    await expect.poll(() => reader.admitted(writer.writer())).toBe(true);
    expect(sha(await download(open, row))).toBe(sha(PNG));
    await row.getByRole('button', { name: text.attachmentPreview }).click();
    await expect(open.getByTestId('file-preview').locator('img')).toHaveAttribute(
      'src',
      /^data:image\/png;base64,/,
    );
  });
});
